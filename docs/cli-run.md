# CLI — `ai-orchestrator run`

## Descrição

Executa o fluxo completo do orquestrador: lê o plano, carrega o contexto, solicita implementação para a IA Dev, valida a resposta, salva o patch, executa o `git apply --check`, cria o handoff e solicita auditoria.

Ao final, exibe o resultado da auditoria para aprovação humana.

Nenhuma alteração é aplicada automaticamente.

---

## Uso

```bash
ai-orchestrator run
```

---

## Fluxo executado

```txt
1. Ler plan.md
2. Carregar memory/context.md + último handoff relevante
3. Enviar request para IA Dev (mailbox/dev-request.json)
4. Receber e validar resposta da IA Dev (mailbox/dev-response.json)
5. Salvar patch em patches/
6. Executar git apply --check
7. Criar handoff dev → auditor (handoffs/dev-to-auditor.json)
8. Sincronizar handoff com IA-Memory
9. Enviar request para IA Auditora (mailbox/audit-request.json)
10. Receber e validar resposta da auditoria (mailbox/audit-response.json)
11. Exibir resultado ao usuário
12. Registrar log em logs/
```

---

## Bloqueios automáticos

O fluxo é interrompido imediatamente se qualquer das condições abaixo for detectada:

| Condição | Motivo |
|---|---|
| JSON inválido na resposta da IA Dev | Schema inválido |
| Campos obrigatórios ausentes | Schema inválido |
| Diff ausente ou vazio | Patch inválido |
| Diff não é Unified Diff | Formato inválido |
| Segredo detectado no diff (`.env`, `API_KEY`, `TOKEN`, `PASSWORD`, `SECRET`) | Violação de segurança |
| Arquivo fora do escopo do plano | Violação de escopo |
| `git apply --check` falha | Patch inaplicável |

---

## Flags

| Flag | Descrição |
|---|---|
| `--step <id>` | Executa apenas o step indicado do plano |
| `--skip-audit` | ⚠️ Pula a auditoria (apenas para desenvolvimento/debug) |
| `--dry-run` | Executa o fluxo sem salvar arquivos nem chamar agentes |

---

## Arquivos gerados por execução

| Arquivo | Conteúdo |
|---|---|
| `patches/proposal-<step_id>.diff` | Patch proposto pela IA Dev |
| `handoffs/dev-to-auditor.json` | Handoff do dev para o auditor |
| `mailbox/audit-request.json` | Contexto enviado para a IA Auditora |
| `mailbox/audit-response.json` | Resposta da IA Auditora |
| `logs/run-<timestamp>.json` | Log completo da execução |

---

## Formato do log gerado

```json
{
  "timestamp": "",
  "plan_hash": "",
  "memory_hash": "",
  "diff_hash": "",
  "dev_response": {},
  "audit_response": {},
  "handoffs": [],
  "final_decision": ""
}
```

---

## Exemplo de saída (aprovado)

```txt
→ Lendo plan.md...
→ Carregando contexto IA-Memory...
→ Solicitando implementação para IA Dev...
✔ Resposta recebida e validada.
✔ Patch salvo em patches/proposal-001.diff
✔ git apply --check passou.
→ Criando handoff dev → auditor...
→ Sincronizando com IA-Memory...
→ Solicitando auditoria...

══════════════════════════════════════
RESULTADO DA AUDITORIA
══════════════════════════════════════
Status  : APROVADO
Score   : 92
Arquivos: src/auth.ts, src/auth.test.ts
Riscos  : Nenhum crítico identificado
══════════════════════════════════════

Execute `ai-orchestrator apply` para aplicar o patch após revisão.
```

---

## Exemplo de saída (reprovado)

```txt
→ Solicitando auditoria...

══════════════════════════════════════
RESULTADO DA AUDITORIA
══════════════════════════════════════
Status  : REPROVADO
Score   : 34
Problemas:
  - Função sem tratamento de erro
  - Arquivo fora do escopo do plano: src/unrelated.ts
Motivo do bloqueio: escopo violado
══════════════════════════════════════

Handoff auditor → dev criado.
Execute `ai-orchestrator run` novamente após correção do plano ou contexto.
```
