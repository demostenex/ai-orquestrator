# North Star — AI Orchestrator

> Documento de visão e direção estratégica do projeto.  
> Revisado em: 2026-05-29

---

## Visão

Um orquestrador de IAs multi-agente com **CLI estável**, capaz de coordenar múltiplos CLIs de IA (Copilot, Gemini, Claude, etc.) sobre um mesmo projeto de software — de forma auditável, previsível e com controle humano em cada transição.

---

## Princípios Fundadores

| # | Princípio | Descrição |
|---|-----------|-----------|
| 1 | **CLI como cidadão de primeira classe** | Interface estável, ergonômica e consistente — inspirada em ferramentas como `gh`, `copilot` e `gemini` CLI |
| 2 | **Sessão persistente por padrão** | CLIs externos nunca perdem sessão a menos que o usuário solicite explicitamente |
| 3 | **Humano no loop** | O usuário decide a cada transição: enriquecer, aprovar, devolver ou avançar |
| 4 | **Sem auto-aplicação** | Nenhuma IA aplica código diretamente — toda mudança passa por plano → dev → auditoria → humano → git |
| 5 | **Auditabilidade total** | Cada interação é registrada em SQLite com rastreabilidade completa |

---

## Modos de Operação

### Modo Planejamento (`plan`)

Duas ou mais IAs interagem sobre o mesmo plano — **uma de cada vez**, mediadas pelo orquestrador.

**Fluxo:**
```
Usuário inicia plano
  ↓
IA Arquiteta (turno 1) — analisa, propõe estrutura
  ↓
Orquestrador apresenta resultado → Usuário enriquece / aprova
  ↓
IA Dev (turno 2) — detalha tarefas e gera todo list
  ↓
Orquestrador apresenta resultado → Usuário enriquece / aprova
  ↓
Plano finalizado → Todo List salvo em arquivo + SQLite
```

**Regras do Modo Planejamento:**
- Cada turno de IA é salvo no SQLite do ai-orchestrator (1 interação por vez)
- Após o plano ser finalizado, o resumo é enviado para o **ai-memory** para persistência
- O Todo List gerado é salvo em arquivo (`.ai-orchestrator/todos/plan-<id>.md`)
- **Somente a IA Dev pode escrever no todo list após o plano finalizado** (o arquivo fica read-only para as demais IAs)

---

### Modo Dev (`dev`)

Uma IA escreve testes e código. Outra IA audita. O usuário controla cada transição.

**Fluxo:**
```
IA Dev recebe: plano + todo list + contexto ai-memory
  ↓
IA Dev produz: diff (Unified Diff) + resumo + riscos
  ↓
Orquestrador gera prompt de transição para o usuário
  ↓
Usuário: enriquece o prompt ou aprova como está
  ↓
IA Auditora recebe: diff + plano + handoff + resultado git apply --check
  ↓
IA Auditora produz: aprovação ou reprovação com feedback
  ↓
Orquestrador apresenta resultado ao usuário
  ↓
Usuário decide: aplicar / devolver para IA Dev / encerrar
```

---

## Gestão de Sessão dos CLIs

- O orquestrador mantém sessões ativas de cada CLI externo (Copilot, Gemini, Claude, etc.)
- Sessões **não são encerradas automaticamente** — persistem entre comandos
- O usuário pode encerrar sessões explicitamente: `ai-orchestrator session kill <cli-name>`
- O orquestrador detecta se uma sessão caiu e oferece retomada antes de recriar

---

## Prompt de Transição (Handoff para o Usuário)

Ao final de cada turno de IA, o orquestrador exibe um **prompt de transição** estruturado:

```
╔══════════════════════════════════════════════╗
║  IA Dev — Turno concluído                    ║
╠══════════════════════════════════════════════╣
║  Resumo: [resumo gerado pela IA]             ║
║  Arquivos tocados: [lista]                   ║
║  Riscos: [lista]                             ║
╠══════════════════════════════════════════════╣
║  O que deseja fazer?                         ║
║  [1] Enviar para IA Auditora (como está)     ║
║  [2] Enriquecer o prompt e enviar            ║
║  [3] Devolver para IA Dev com instrução      ║
║  [4] Encerrar e revisar manualmente          ║
╚══════════════════════════════════════════════╝
```

---

## Persistência e Memória

| Camada | Armazenamento | Conteúdo |
|--------|--------------|----------|
| **Operacional (SSOT)** | SQLite (`.ai-orchestrator/history.db`) | Fonte da verdade para Runs, handoffs, interações, **Todo Lists** e Planos. |
| **Visualização** | Markdown (`plan.md`) | Exportação **read-only** do estado atual do SQLite para consumo humano e IA Auditora. |
| **Memória de longo prazo** | ai-memory (`memory/context.md`) | Decisões arquiteturais, riscos, aprendizados persistentes. |

**Regra de sincronização:**  
Ao finalizar um plano ou ciclo dev → auditoria, o orquestrador extrai o resumo e sincroniza com ai-memory. O SQLite é o mestre; qualquer arquivo Markdown é derivado e recriado automaticamente em caso de divergência.

---

## Gestão de Sessão (CLI Persistente)

Para garantir que IAs externas não percam o contexto da conversa, o orquestrador gerencia processos via **PTY (Pseudo-Terminal)**.

- **PTY Pool:** Mantém o CLI (Claude, Gemini, etc.) aberto em background.
- **Isolamento:** Cada `run_id` pode ter sua própria sessão persistente.
- **Controle:** Comandos para listar, encerrar ou retomar sessões (`ai-orchestrator session ...`).

---

## Transição e Handoff (TUI Menu)

As transições entre agentes não são silenciosas. O orquestrador utiliza uma **Interface TUI (Terminal User Interface)** para o "Portão Humano":

1. **Menu Interativo:** Seleção via setas (Aprovar, Enriquecer, Devolver, Abortar).
2. **Visualização de Diff:** Ver o patch antes de enviar para a Auditora.
3. **Injeção de Contexto:** Notas adicionadas pelo usuário no gate são injetadas no campo `user_modifications` do handoff e tornam-se mandatórias para a validação da Auditora.

---

## Roadmap de Features

### V1 — Base Estável e Robusta _(em andamento)_
- [x] CLI com subcomandos base
- [x] Contratos de handoff JSON
- [ ] **SQLite como Single Source of Truth (Planos + Tasks)**
- [ ] **Exportação automática de Markdown (Read-only)**
- [ ] **Interface TUI para transições (inquire/dialoguer)**
- [ ] **Gerenciamento de processos via PTY (portable-pty)**
- [ ] **Injeção de modificações do usuário no contexto da Auditora**

### V2 — Experiência e Integração
- [ ] CLI interativo estilo REPL
- [ ] Suporte a múltiplos CLIs simultâneos
- [ ] Sincronização automática com ai-memory ao fechar ciclo
- [ ] Dashboard de status em tempo real

---

## Restrições Não Negociáveis

- Nenhuma IA aplica código diretamente
- Segredos (`.env`, `API_KEY`, `TOKEN`, `PASSWORD`, `SECRET`) bloqueiam o fluxo em qualquer etapa
- Todo handoff deve passar por `git apply --check` antes da auditoria
- O usuário sempre tem a palavra final antes de qualquer `git apply`
- Somente a IA Dev escreve no todo list após o plano ser aprovado

---

## Referências Internas

- `agents.md` — contratos de cada agente
- `docs/cli-*.md` — documentação dos subcomandos
- `.ai-orchestrator/memory/context.md` — contexto persistente (ai-memory)
