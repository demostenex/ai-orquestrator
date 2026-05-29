# Agentes — AI Orchestrator Multi-Agente

## Visão Geral

O sistema coordena múltiplas IAs operando sobre o mesmo projeto de software de forma auditável, previsível e segura. Nenhuma IA possui autonomia para aplicar código diretamente — toda alteração passa por plano, handoff, auditoria e aprovação humana.

---

## Agentes Disponíveis

### IA Dev

**Responsabilidade:** Implementação.

Recebe um plano e um contexto, produz um diff em formato Unified Diff e gera o handoff para o auditor.

**Contrato de resposta obrigatório:**

```json
{
  "step_id": "001",
  "summary": "",
  "files_touched": [],
  "diff": "",
  "tests_suggested": [],
  "risks": []
}
```

**Restrições:**
- Não pode alterar arquivos fora do escopo definido no plano.
- Não pode incluir segredos, tokens, senhas ou variáveis de ambiente no diff.
- Deve gerar handoff `dev → auditor` após cada resposta.

---

### IA Auditora

**Responsabilidade:** Revisão e aprovação/reprovação de patches.

Recebe plano, contexto IA-Memory, handoff do dev, diff e resultado do `git apply --check`.

**Contrato de resposta obrigatório:**

```json
{
  "approved": false,
  "score": 0,
  "problems": [],
  "required_changes": [],
  "blocked_reason": null
}
```

**Restrições:**
- Deve bloquear qualquer patch com segredos detectados.
- Deve bloquear patches fora do escopo do plano.
- Deve bloquear patches que falhem no `git apply --check`.
- Deve gerar handoff `auditor → dev` em caso de reprovação.

---

### IA Arquiteta _(opcional)_

**Responsabilidade:** Decisões arquiteturais antes da implementação.

Opera entre o plano e a IA Dev. Pode atualizar `memory/context.md` com decisões e restrições arquiteturais.

**Quando usar:**
- Tarefas com alto impacto arquitetural.
- Refatorações que afetam múltiplos módulos.
- Introdução de novos padrões ou dependências.

**Contrato de handoff `arquiteto → dev`:**

```json
{
  "agent": "architect",
  "target_agent": "dev",
  "step_id": "001",
  "status": "ready_for_dev",
  "summary": "",
  "decisions": [],
  "constraints": [],
  "open_questions": [],
  "risks": [],
  "next_action": ""
}
```

---

### IA-Memory

**Responsabilidade:** Memória operacional compartilhada entre todos os agentes.

Armazena e disponibiliza contexto persistente ao longo do ciclo de vida do projeto.

**O que armazena:**
- Decisões arquiteturais
- Restrições de negócio
- Contexto relevante de execuções anteriores
- Aprendizados e riscos registrados

**O que não armazena:**
- Diffs completos
- Segredos, tokens, senhas
- Arquivos de código extensos

**Sincronização:**

Após cada handoff, o orquestrador extrai o resumo e envia apenas:
- Resumo da decisão
- Riscos identificados
- Dúvidas em aberto
- Contexto relevante

O arquivo local de referência é:

```txt
.ai-orchestrator/memory/context.md
```

---

## Fluxo de Responsabilidade

```txt
Plano
  ↓
IA Arquiteta (opcional)
  ↓
IA Dev
  ↓
IA Auditora
  ↓
Humano
  ↓
Git
```

---

## Contrato de Handoff

Todo agente **obrigatoriamente** gera um handoff ao concluir sua etapa.

Nenhum agente pode iniciar trabalho sem ler:
- `plan.md`
- Contexto IA-Memory (`memory/context.md`)
- Último handoff relevante
- Status Git atual

**Formato padrão de handoff:**

```json
{
  "agent": "",
  "target_agent": "",
  "step_id": "",
  "status": "",
  "summary": "",
  "decisions": [],
  "files_touched": [],
  "open_questions": [],
  "risks": [],
  "next_action": ""
}
```

**Valores válidos para `status`:**

| Status | Descrição |
|---|---|
| `waiting_audit` | Dev concluiu, aguarda auditoria |
| `approved` | Auditoria aprovou |
| `rejected` | Auditoria reprovou |
| `ready_for_dev` | Arquiteto concluiu, dev pode iniciar |
| `waiting_human` | Aguarda aprovação humana |

---

## Regras Globais dos Agentes

| Regra | Detalhe |
|---|---|
| Sem auto-aplicação | Nenhum agente aplica código diretamente |
| Handoff obrigatório | Todo agente deve gerar handoff ao concluir |
| Leitura de contexto obrigatória | Todo agente lê plano + contexto + último handoff antes de iniciar |
| Sem segredos | Qualquer ocorrência de `.env`, `API_KEY`, `TOKEN`, `PASSWORD`, `SECRET` bloqueia o fluxo |
| Escopo restrito | Alterações fora do plano são bloqueadas |
| Diff válido | Somente Unified Diff é aceito |
| Git check obrigatório | `git apply --check` deve passar antes da auditoria |

---

## Preparação para MCP (Futuro)

Na V1 não há integração MCP. A arquitetura foi pensada para suportar futuramente:

```txt
audit_dev_output
create_handoff
sync_memory
load_context
review_plan
```

<!-- ai-memory:start -->
## Long-term memory (ai-memory)

This project uses [ai-memory](https://github.com/akitaonrails/ai-memory)
for cross-session continuity.

**Default to the current project — always.** Every ai-memory tool
auto-scopes to the project resolved from your session's working
directory. **Do NOT pass `project` or `cwd` arguments unless the user
explicitly references a *different* project by name** (e.g. "what did we
decide in the `other-app` project?"). Phrases like "this project",
"here", "we", "our work", "where did we leave off" all mean the *current*
project — call the tool with no scoping args. If the user asks about a
handoff and the SessionStart auto-fetched block is already in your
context, just answer from it; do not re-call the tool to "find it again"
in another project.

**Lifecycle hooks already capture every prompt + tool call
automatically.** You never need to manually write routine notes; the
SessionStart hook auto-fetches pending handoffs, and on session end
ai-memory writes a session-summary page and a handoff.
LLM consolidation (compiling observations into topical wiki pages) runs
on PreCompact, on demand via `memory_consolidate`, and at session end
only when the server sets `AI_MEMORY_CONSOLIDATE_ON_SESSION_END`. Only
write a durable wiki page when the user explicitly asks to remember or
annotate something permanently.

### When to reach for each tool

The user can express any of the intents below in plain English —
match the intent to the tool. They do not need to name the tool.

| User says / situation | Tool |
|---|---|
| "have we discussed X?" / "search memory for Y\" / before proposing architecture | `memory_query` |
| "what's been going on" / "show recent activity" (light) | `memory_recent` |
| "is ai-memory healthy?" / "how big is the wiki?" | `memory_status` |
| "give me the stats" / structured snapshot for the agent to consume | `memory_briefing` |
| "catch me up" / "I've been away" / "what's important right now?" / open-ended exploration | `memory_explore` |
| "where did we leave off?" — and you see a `📥 ai-memory: pending handoff` block in your context | already done — answer from that block; do NOT re-call `memory_handoff_accept` |
| "where did we leave off?" — and no such block is visible | `memory_handoff_accept` (rare; the SessionStart hook usually got there first) |
| "save context for the next session" / wrapping up | `memory_handoff_begin` (single-use handoff; terse summary; put detail in `open_questions` + `next_steps` bullets) |
| "consolidate this session" / "compile what we learned" (also runs on PreCompact; at session end only if `AI_MEMORY_CONSOLIDATE_ON_SESSION_END` is set) | `memory_consolidate` |
| "remember this permanently" / "save a note" / "add an annotation" / durable project knowledge | `memory_write_page` (write a wiki page; do **not** use handoff for permanent notes) |
| "read the page about X" / "show me the full content of Y" / "open the page on Z" | `memory_read_page` (full body; pass a query to search or `path` for a direct lookup) |
| "audit the wiki" / "find contradictions" / "what rules should we add?" | `memory_lint` |
| "prune old pages" / "memory cleanup" | `memory_forget_sweep` |

`memory_explore` is the right default for the "I want to know what's
going on" use case — it returns a prose digest whose verbosity
scales automatically to how long it's been since the last activity
(< 1 h → one line; > 30 days → full catchup).

### When you write a project rule, write it here

If you're about to write a durable project rule ("always X", "never
Y", "all PRs must …"), this rules file (CLAUDE.md for Claude Code;
AGENTS.md for Codex / OpenCode / Cursor / Gemini CLI; whichever
convention your agent uses) is where it belongs. ai-memory's lint
pass surfaces the same hint automatically when a `kind: rule` page
lands in `_rules/`.

### Refreshing this snippet

This block is maintained by ai-memory. Two ways to refresh it with
the latest binary's recommended copy:

- **From the agent** (no terminal needed): ask "refresh the ai-memory
  routing in this project" — the agent calls
  `memory_install_self_routing`, picks the right filename for itself
  (Claude Code → `CLAUDE.md`; Codex / OpenCode / Cursor / Gemini →
  `AGENTS.md`), and uses its Write / Edit tool to land the block.
- **From the CLI**: `ai-memory install-instructions` (defaults to
  `CLAUDE.md`; pass `--target AGENTS.md` for non-Claude agents).

Both are idempotent: re-runs replace the block bracketed by
`<!-- ai-memory:start -->` / `<!-- ai-memory:end -->` markers
without disturbing the rest of the file.
<!-- ai-memory:end -->
