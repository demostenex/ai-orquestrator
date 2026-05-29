# CLI — `ai-orchestrator audit`

## Descrição

Executa apenas a etapa de auditoria sobre um patch já existente em `patches/`.

Útil para re-auditar um patch após correções manuais ou para auditar um patch gerado externamente.

Não solicita nova implementação para a IA Dev.

---

## Uso

```bash
ai-orchestrator audit
```

```bash
ai-orchestrator audit --patch patches/proposal-001.diff
```

---

## O que faz

1. Carrega o patch indicado (ou o mais recente em `patches/`).
2. Executa `git apply --check` sobre o patch.
3. Carrega `plan.md`, `memory/context.md` e o último handoff relevante.
4. Envia contexto + patch para a IA Auditora.
5. Valida o schema da resposta.
6. Exibe o resultado da auditoria.
7. Registra log em `logs/`.

---

## Bloqueios automáticos

| Condição | Comportamento |
|---|---|
| Nenhum patch encontrado em `patches/` | Aborta com erro |
| `git apply --check` falha | Aborta antes de chamar a auditora |
| Schema da resposta da auditora inválido | Aborta e registra erro no log |
| Segredo detectado no patch | Aborta imediatamente |

---

## Flags

| Flag | Descrição |
|---|---|
| `--patch <caminho>` | Especifica o patch a auditar. Se omitido, usa o mais recente em `patches/` |
| `--step <id>` | Filtra pelo step_id do handoff |
| `--dry-run` | Exibe o contexto que seria enviado para a auditora sem chamá-la |

---

## Contrato de resposta esperado da IA Auditora

```json
{
  "approved": false,
  "score": 0,
  "problems": [],
  "required_changes": [],
  "blocked_reason": null
}
```

| Campo | Tipo | Obrigatório |
|---|---|---|
| `approved` | boolean | ✅ |
| `score` | number (0–100) | ✅ |
| `problems` | string[] | ✅ |
| `required_changes` | string[] | ✅ |
| `blocked_reason` | string \| null | ✅ |

---

## Arquivos gerados

| Arquivo | Conteúdo |
|---|---|
| `mailbox/audit-request.json` | Contexto enviado para a auditora |
| `mailbox/audit-response.json` | Resposta da auditora |
| `handoffs/auditor-to-dev.json` | Criado automaticamente em caso de reprovação |
| `logs/audit-<timestamp>.json` | Log da execução |

---

## Exemplo de saída

```txt
→ Carregando patch: patches/proposal-001.diff
✔ git apply --check passou.
→ Carregando contexto...
→ Solicitando auditoria...

══════════════════════════════════════
RESULTADO DA AUDITORIA
══════════════════════════════════════
Status  : APROVADO
Score   : 88
Arquivos: src/payment.ts
Problemas: Nenhum
══════════════════════════════════════

Execute `ai-orchestrator apply` para aplicar o patch após revisão.
```
