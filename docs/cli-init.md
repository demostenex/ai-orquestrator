# CLI — `ai-orchestrator init`

## Descrição

Inicializa a estrutura de diretórios e arquivos necessários para o AI Orchestrator no projeto atual.

Deve ser executado uma única vez na raiz do repositório antes de qualquer outro comando.

---

## Uso

```bash
ai-orchestrator init
```

---

## O que faz

1. Cria a estrutura de diretórios `.ai-orchestrator/`.
2. Gera o arquivo `plan.md` com template inicial.
3. Gera o arquivo `memory/context.md` com template inicial.
4. Cria as pastas `mailbox/`, `handoffs/`, `patches/`, `logs/` e `memory-sync/`.
5. Verifica se o diretório já é um repositório Git. Emite aviso se não for.

---

## Estrutura gerada

```txt
.ai-orchestrator/
│
├── plan.md
│
├── memory/
│   └── context.md
│
├── mailbox/
│   ├── dev-request.json
│   ├── dev-response.json
│   ├── audit-request.json
│   └── audit-response.json
│
├── handoffs/
│   ├── architect-to-dev.json
│   ├── dev-to-auditor.json
│   └── auditor-to-dev.json
│
├── patches/
│
├── logs/
│
└── memory-sync/
    └── last-sync.json
```

---

## Flags

| Flag | Descrição |
|---|---|
| `--force` | Reinicializa mesmo se `.ai-orchestrator/` já existir |
| `--dry-run` | Exibe o que seria criado sem criar nada |

---

## Erros comuns

| Situação | Comportamento |
|---|---|
| `.ai-orchestrator/` já existe | Aborta com mensagem de erro. Use `--force` para reinicializar. |
| Diretório não é um repositório Git | Exibe aviso, mas continua a inicialização |

---

## Exemplo de saída

```txt
✔ Criando estrutura .ai-orchestrator/
✔ plan.md gerado
✔ memory/context.md gerado
✔ mailbox/ criado
✔ handoffs/ criado
✔ patches/ criado
✔ logs/ criado
✔ memory-sync/ criado

AI Orchestrator inicializado com sucesso.
Edite .ai-orchestrator/plan.md para definir o plano antes de executar `ai-orchestrator run`.
```
