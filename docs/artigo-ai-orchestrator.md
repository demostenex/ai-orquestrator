# AI Orchestrator: tentando dar autonomia para IA sem entregar o volante

Usar IA para programar em 2026 é fácil.

Difícil é confiar no que ela acabou de fazer.

Você pede uma correção simples. A IA responde com segurança, entrega um diff bonito, explica que "o problema foi resolvido" e pronto: agora o projeto não compila, o patch não aplica, ou apareceu um `API_KEY` no meio de um arquivo que nunca deveria ter sido tocado.

Não é necessariamente porque a IA é ruim.

É porque a gente costuma usar IA como se ela fosse um desenvolvedor sênior com acesso livre ao teclado. Ela não é. Ela é boa em propor, boa em acelerar, boa em enxergar padrões. Mas ela precisa de cerca, trilho, log, auditoria e humano no final da fila.

Foi daí que nasceu o **AI Orchestrator**.

Não como mais um wrapper bonitinho para chamar OpenAI, Anthropic ou Gemini. Isso qualquer script faz.

A ideia aqui é outra: criar um fluxo onde várias IAs podem trabalhar sobre uma codebase, mas nenhuma delas tem poder para aplicar código sozinha.

# 1. O incômodo: IA não pode ser root do seu projeto

O fluxo normal de muita gente hoje é quase suicida:

```txt
prompt -> resposta -> copia e cola -> torce
```

Às vezes funciona. Às vezes funciona até demais, e esse é o perigo.

Quando a IA acerta nove vezes, você relaxa na décima. E a décima é justamente a que altera um arquivo fora do escopo, quebra uma migração, remove um teste, troca uma regra de negócio ou vaza um segredo.

O problema técnico é simples:

```txt
autonomia sem auditoria vira dívida técnica automática
```

Então eu desenhei o AI Orchestrator com uma regra básica:

> A IA pode propor. Quem aplica é o fluxo. Quem autoriza é o humano.

Isso muda tudo.

# 2. O North Star: plano, diff, auditoria, humano e Git

Antes de pensar em TUI, SQLite, Ratatui ou provider de LLM, eu precisava definir a fronteira moral do projeto.

A fronteira ficou assim:

```txt
Plano
  -> IA Dev
  -> Handoff
  -> IA Auditora
  -> git apply --check
  -> aprovação humana
  -> git apply
```

Nenhum agente aplica código diretamente.

O agente Dev recebe contexto, plano e memória. Ele não "edita o projeto". Ele devolve um `Unified Diff`.

A Auditora recebe o plano, o diff, o handoff, a memória e o resultado do `git apply --check`. Se o patch não aplica, se foge do escopo ou se tem cheiro de segredo, bloqueia.

Essa decisão veio de uma constatação bem simples: LLM é boa em gerar intenção, mas intenção não é mudança aplicável. O artefato mínimo confiável precisava ser um patch, porque patch pode ser inspecionado, hasheado, salvo, auditado e testado antes de tocar no disco.

Só depois disso o humano vê o que aconteceu e decide.

É menos mágico? Sim.

Mas mágica em ambiente de produção geralmente é só falta de log com outro nome.

# 3. Rust: porque a ferramenta que mexe no seu código não pode ser frágil

Escolhi Rust por um motivo bem prático: eu queria uma ferramenta de terminal que fosse difícil de quebrar pelos motivos errados.

Não era sobre hype. Era sobre construir uma base que aguenta:

- CLI com comandos previsíveis.
- TUI com estado interno sem virar uma sopa.
- SQLite embutido.
- execução de processos externos.
- validação de diff.
- integração com ferramentas como `ai-memory`.

O `Cargo.toml` acabou ficando bem honesto sobre o tipo de problema:

- `ratatui` e `crossterm` para a interface no terminal.
- `portable-pty` para conversar com CLIs de IA como processo interativo.
- `rusqlite` com SQLite bundled para histórico local.
- `serde` e `serde_json` para contratos de agente.
- `regex` para varredura de segredos.
- `sha2` para hash de patches e eventos.

Isso não é stack de landing page. É stack de ferramenta que precisa sobreviver ao terminal.

Um exemplo dessa decisão é o uso de PTY. Eu poderia chamar cada CLI de IA como processo descartável, mandar um prompt, ler a resposta e matar o processo. Seria mais simples. Mas perderia o estado conversacional e ficaria mais difícil simular uma sessão real de terminal.

Por isso existe uma camada de sessão:

```rust
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};

pub struct PtySession {
    child: Box<dyn portable_pty::Child + Send + Sync>,
}
```

O motivo não é glamour técnico. É pragmatismo. Se a ferramenta precisa orquestrar CLIs externas, ela precisa falar a língua delas: stdin, stdout, terminal, estado e tempo de resposta.

# 4. SQLite: Markdown é bom para ler, não para ser fonte da verdade

No começo, a tentação é salvar tudo em arquivo Markdown ou JSON solto.

É simples. É visível. É fácil de debugar.

Mas quando você começa a ter plano, tarefas, eventos, handoffs, auditoria, violações de segurança e histórico de execução, arquivo solto começa a cobrar juros.

Por isso o AI Orchestrator usa SQLite como fonte operacional da verdade.

O Markdown continua existindo, mas como exportação legível. Quem manda é o banco:

```txt
.ai-orchestrator/history.db
```

O banco guarda runs, eventos, handoffs, violações e recibos de sincronização com `ai-memory`. O modo WAL ajuda a manter escrita e leitura convivendo sem transformar tudo em gargalo.

No código, a abertura do banco já força essa decisão:

```rust
let db_path = orchestrator_dir.join("history.db");

let manager = SqliteConnectionManager::file(&db_path)
    .with_init(|c| c.pragma_update(None, "journal_mode", "WAL"));

let pool = Pool::new(manager)?;

let conn = pool.get()?;
conn.execute_batch(SCHEMA)?;
```

Por que WAL? Porque a TUI lê estado enquanto o fluxo escreve evento. O painel quer mostrar o que está acontecendo agora, enquanto o executor registra resposta, patch, hash, auditoria e bloqueio. Sem isso, o banco vira um ponto de atrito justamente no lugar onde eu preciso de fluidez.

Aqui tem uma decisão importante: o humano precisa conseguir ler o estado do projeto, mas a máquina precisa conseguir confiar nele.

Markdown resolve a primeira parte.

SQLite resolve a segunda.

# 5. Handoff: o bastão entre IAs

Uma IA falando com outra sem contrato vira reunião ruim.

Então cada transição do fluxo gera um handoff estruturado. O Dev não manda apenas "fiz tal coisa". Ele precisa dizer:

- qual tarefa executou;
- quais arquivos tocou;
- qual decisão tomou;
- quais riscos existem;
- o que a próxima IA deve fazer.

O Auditor também responde em contrato. Ele aprova, reprova, dá score, lista problemas e aponta mudanças obrigatórias.

O contrato do Dev ficou explícito no schema:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevResponse {
    pub step_id: String,
    pub summary: String,
    pub files_touched: Vec<String>,
    pub diff: String,
    pub tests_suggested: Vec<String>,
    pub risks: Vec<String>,
}
```

E o handoff não é texto livre. Ele é criado a partir da resposta do Dev e carrega o hash do patch como uma decisão:

```rust
pub fn create_dev_to_auditor_handoff(
    _run_id: &str,
    step_id: &str,
    dev_response: &DevResponse,
    patch_hash: &str,
) -> Handoff {
    let mut decisions = vec![format!("Patch hash: {patch_hash}")];

    Handoff {
        agent: "dev".to_string(),
        target_agent: "auditor".to_string(),
        step_id: step_id.to_string(),
        status: HandoffStatus::WaitingAudit,
        summary: dev_response.summary.clone(),
        decisions,
        files_touched: dev_response.files_touched.clone(),
        risks: dev_response.risks.clone(),
        next_action: "Revisar o patch, validar escopo e auditar riscos de segurança.".to_string(),
        open_questions: Vec::new(),
    }
}
```

O motivo dessa escolha é rastreabilidade. Se a Auditora aprovou um patch, eu quero saber qual patch. Não "um patch parecido", não "a última resposta do modelo", mas aquele conteúdo específico, com hash.

Isso parece burocracia até a primeira vez em que você precisa entender por que um patch foi bloqueado.

Sem handoff, você tem conversa.

Com handoff, você tem rastro.

# 6. O filtro de segredos é um hard-stop

Esse ponto não é negociável.

Se o diff contém padrão de `API_KEY`, `TOKEN`, `PASSWORD`, `SECRET` ou coisa parecida, o fluxo trava.

Não pergunta se "talvez seja exemplo".

Não tenta ser esperto.

Não aplica.

Ferramenta de automação que mexe com código precisa ser conservadora por padrão. O custo de bloquear um falso positivo é irritação. O custo de deixar passar um segredo é incidente.

Essa é uma troca fácil.

O scanner é propositalmente direto:

```rust
pub fn scan_diff(content: &str) -> Vec<SecurityViolation> {
    let detectors = [
        (Regex::new(r"\bAPI_KEY\b").expect("valid regex"), "API_KEY"),
        (Regex::new(r"\bTOKEN\b").expect("valid regex"), "TOKEN"),
        (Regex::new(r"\bPASSWORD\b").expect("valid regex"), "PASSWORD"),
        (Regex::new(r"\bSECRET\b").expect("valid regex"), "SECRET"),
        (Regex::new(r"\bPRIVATE_KEY\b").expect("valid regex"), "PRIVATE_KEY"),
    ];

    let mut violations = Vec::new();

    for (index, line) in content.lines().enumerate() {
        let line_number = index + 1;
        for (regex, label) in &detectors {
            if regex.is_match(line) {
                violations.push(SecurityViolation {
                    pattern: (*label).to_string(),
                    line_number,
                });
            }
        }
    }

    violations
}
```

E no fluxo de execução isso vira bloqueio, não aviso decorativo:

```rust
let violations = scan_diff(&dev_response.diff);
if !violations.is_empty() {
    for v in &violations {
        db.log_event(
            EventType::SecurityBlocked,
            Some("security"),
            None,
            Some(&format!("{:?}", v)),
            None,
            false,
            None,
        )
        .await?;
    }
    return Err(anyhow!("security violations detected"));
}
```

Aqui a decisão foi bem consciente: segurança não entra como "recomendação da auditora". Entra como porta fechada antes da auditoria.

# 7. A TUI: porque terminal não precisa ser cego

O projeto começou como CLI, mas CLI pura tem um limite: você executa comando, recebe texto, executa outro comando, recebe mais texto, e precisa montar o estado inteiro na cabeça.

Com Ratatui, o AI Orchestrator ganhou uma cara mais operacional.

A ideia da TUI não é ser bonita por ser bonita. É deixar claro:

- qual plano está ativo;
- quais tarefas existem;
- qual agente está rodando;
- quais eventos aconteceram;
- qual handoff está disponível;
- o que veio do `ai-memory`;
- onde o fluxo está esperando o humano.

Isso muda a sensação de uso. Em vez de "chamei uma IA e espero que dê certo", você passa a operar um painel de controle.

Ainda é terminal. Só que agora o terminal mostra o processo, não apenas o resultado.

# 8. ai-memory: contexto longo sem enfiar o repositório inteiro no prompt

Contexto é caro.

Mandar o projeto inteiro para a IA a cada turno é lento, caro e muitas vezes piora a resposta. A IA passa a nadar em ruído.

O AI Orchestrator usa `ai-memory` para manter uma memória de longo prazo: decisões, riscos, aprendizados e handoffs importantes.

O ponto não é substituir o código. O código continua sendo a fonte do que o sistema faz.

O `ai-memory` serve para responder outra pergunta:

> O que a gente já decidiu sobre este projeto?

Isso é especialmente útil em fluxo multi-agente. O Dev não precisa redescobrir tudo. O Auditor não precisa depender só do diff. E o humano não precisa repetir o mesmo contexto em toda execução.

O prompt do Dev deixa isso explícito:

```rust
format!(
    "Step atual: {step_id}\n\n\
Contexto obrigatório:\n\
[PLAN]\n{plan}\n\n\
[MEMORY]\n{memory}\n\n\
[WORKSPACE]\n{workspace_snapshot}\n\n\
[LAST_HANDOFF]\n{handoff_text}\n\n\
Regras: gere apenas diff unified, respeite o escopo do plano, \
não inclua segredos e não explique nada fora do JSON solicitado."
)
```

Repara na ordem: plano, memória, workspace e último handoff. Eu não quero que a IA "se inspire". Quero que ela trabalhe dentro de um contexto delimitado.

E quando faz sentido persistir algo para a próxima rodada, o orquestrador escreve no `ai-memory` como página, não como um blob perdido:

```rust
let mut child = std::process::Command::new("ai-memory")
    .args([
        "write-page",
        "--path",
        wiki_path,
        "--title",
        title,
        "--body",
        "-",
    ])
    .stdin(std::process::Stdio::piped())
    .stdout(std::process::Stdio::piped())
    .spawn()
    .ok()?;
```

O motivo da decisão: memória operacional fica no SQLite; memória durável, que precisa atravessar sessões e agentes, vai para o `ai-memory`.

# 9. O fluxo real

Na prática, uma execução saudável se parece com isso:

```txt
1. Você cria ou retoma um plano.
2. O orquestrador carrega memória, status Git e último handoff.
3. A IA Dev recebe uma tarefa específica.
4. Ela devolve JSON com resumo, riscos, arquivos tocados e diff.
5. O orquestrador varre segredos.
6. O orquestrador roda git apply --check.
7. A IA Auditora revisa escopo, segurança e aplicabilidade.
8. O humano aprova ou recusa.
9. Só então o patch toca o disco.
```

Repare no detalhe: a parte mais importante do sistema não é chamar IA.

É dizer "não" para ela quando precisa.

Esse trecho do executor resume bem o espírito:

```rust
let parsed_diff = parse_diff(&dev_response.diff)?;
let patch_hash = compute_patch_hash(&parsed_diff.raw);

write_string(&patch_path, &parsed_diff.raw)?;

git::apply_check(&config.workspace_dir, &patch_path)?;
print_success("git apply --check passou.");
```

Eu salvo o patch, calculo hash e testo aplicabilidade antes de seguir. A IA pode ter escrito o melhor texto do mundo; se o diff não aplica, acabou a conversa.

# 10. O que aprendi construindo isso

Primeiro: diff gerado por IA é um bicho sensível.

Às vezes a ideia está certa, mas o contexto do patch está errado. Às vezes o patch foi gerado sobre uma versão antiga. Às vezes a IA troca um trecho que parece igual, mas não é.

Por isso `git apply --check` virou etapa obrigatória, não sugestão.

Segundo: agente sem contrato vira prompt solto.

Quando cada IA responde no formato que quer, o orquestrador vira um parser de desculpas. JSON obrigatório não é glamour, mas salva o fluxo.

Terceiro: auditoria precisa acontecer antes da emoção.

Se você vê um patch grande e ele "parece bom", a vontade é aplicar logo. O sistema existe justamente para colocar atrito onde o nosso julgamento costuma ficar preguiçoso.

# 11. Trade-off: menos velocidade aparente, mais confiança real

Sim, o AI Orchestrator adiciona etapas.

Tem plano. Tem handoff. Tem auditoria. Tem checagem de segurança. Tem confirmação humana.

Para um script descartável, talvez seja exagero.

Para uma codebase que você quer manter viva, não é.

A promessa não é fazer a IA codar mais rápido em linha reta. A promessa é reduzir retrabalho, susto e sujeira depois.

Eu não quero uma IA que saia alterando arquivo como se fosse dona do repositório.

Eu quero uma IA trabalhando dentro de um processo que eu consigo auditar.

# Conclusão: autonomia precisa de freio

O AI Orchestrator é uma tentativa de tratar IA como força produtiva, mas não como autoridade.

Ela escreve. Ela propõe. Ela revisa. Ela discorda.

Mas não aplica sozinha.

No fim, o projeto é menos sobre "multi-agente" e mais sobre responsabilidade. Se a IA vai participar do desenvolvimento de software de verdade, ela precisa deixar rastro, respeitar escopo e aceitar veto.

Autonomia sem controle é só terceirização da bagunça.

Autonomia com auditoria começa a parecer engenharia.

Próximos passos: amadurecer o backend PTY, melhorar os painéis simultâneos no Ratatui e testar uma integração opcional com `tmux` para quem já vive com várias sessões abertas no terminal.
