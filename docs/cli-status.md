# CLI — `ai-orchestrator status`

## Descrição

Exibe o estado atual do orquestrador: plano ativo, último handoff, patches pendentes, resultado da última auditoria e status do repositório Git.

Útil para retomar o trabalho após uma pausa ou para ter uma visão rápida do ciclo em andamento.

---

## Uso

```bash
ai-orchestrator status
```

---

## O que exibe

### Plano

- Hash do `plan.md` atual
- Número de steps definidos
- Step atual em execução (se houver)

### Git

- Branch atual
- Status do working tree (`clean` / `dirty`)
- Último commit (hash + mensagem)

### Patches

- Lista de patches em `patches/` com status:
  - `pending` — sem auditoria
  - `audited:approved` — auditoria aprovada, aguardando apply
  - `audited:rejected` — auditoria reprovada
  - `applied` — patch já aplicado

### Último handoff

- Agente de origem
- Agente de destino
- Status
- Timestamp

### Última auditoria

- Status (`approved` / `rejected`)
- Score
- Problemas encontrados (resumo)

### IA-Memory

- Hash do `memory/context.md` atual
- Timestamp da última sincronização (`memory-sync/last-sync.json`)

---

## Flags

| Flag | Descrição |
|---|---|
| `--json` | Saída em formato JSON estruturado |
| `--short` | Exibe apenas os campos críticos (status, patch, auditoria) |

---

## Exemplo de saída (modo padrão)

```txt
══════════════════════════════════════
AI ORCHESTRATOR — STATUS
══════════════════════════════════════

PLANO
  Arquivo  : .ai-orchestrator/plan.md
  Hash     : sha256:7a3f...
  Steps    : 3 definidos
  Step atual: 001 — Implementar autenticação JWT

GIT
  Branch   : feat/auth
  Status   : clean
  Último commit: a1b2c3d "chore: setup inicial"

PATCHES
  proposal-001.diff → audited:approved  (aguardando apply)
  proposal-000.diff → applied

ÚLTIMO HANDOFF
  De      : dev
  Para    : auditor
  Status  : waiting_audit
  Timestamp: 2025-01-15T10:00:00Z

ÚLTIMA AUDITORIA
  Status  : APROVADO
  Score   : 92
  Problemas: Nenhum

IA-MEMORY
  Hash    : sha256:4d9e...
  Última sync: 2025-01-15T09:58:00Z

══════════════════════════════════════
Próximo passo sugerido: ai-orchestrator apply
══════════════════════════════════════
```

---

## Exemplo de saída (`--json`)

```json
{
  "plan": {
    "file": ".ai-orchestrator/plan.md",
    "hash": "sha256:7a3f...",
    "steps_count": 3,
    "current_step": "001"
  },
  "git": {
    "branch": "feat/auth",
    "status": "clean",
    "last_commit": "a1b2c3d"
  },
  "patches": [
    { "file": "proposal-001.diff", "status": "audited:approved" },
    { "file": "proposal-000.diff", "status": "applied" }
  ],
  "last_handoff": {
    "from": "dev",
    "to": "auditor",
    "status": "waiting_audit",
    "timestamp": "2025-01-15T10:00:00Z"
  },
  "last_audit": {
    "approved": true,
    "score": 92,
    "problems": []
  },
  "memory": {
    "hash": "sha256:4d9e...",
    "last_sync": "2025-01-15T09:58:00Z"
  }
}
```

---

## Estados possíveis do ciclo

| Estado | Próximo comando sugerido |
|---|---|
| Sem plano | `ai-orchestrator init` + editar `plan.md` |
| Plano definido, sem patch | `ai-orchestrator run` |
| Patch pendente, sem auditoria | `ai-orchestrator audit` |
| Patch reprovado | Revisar plano/contexto → `ai-orchestrator run` |
| Patch aprovado | `ai-orchestrator apply` |
| Patch aplicado | Commitar → iniciar próximo step |
