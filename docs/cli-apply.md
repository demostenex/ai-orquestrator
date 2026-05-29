# CLI — `ai-orchestrator apply`

## Descrição

Aplica um patch aprovado ao repositório após confirmação explícita do usuário.

**Nenhum patch é aplicado automaticamente.** A confirmação manual é obrigatória e intencionalmente inconveniente para evitar aplicações acidentais.

---

## Uso

```bash
ai-orchestrator apply
```

```bash
ai-orchestrator apply --patch patches/proposal-001.diff
```

---

## O que faz

1. Carrega o patch indicado (ou o mais recente em `patches/`).
2. Verifica se existe uma auditoria aprovada correspondente em `mailbox/audit-response.json`.
3. Executa novamente `git apply --check` para garantir que o patch ainda é aplicável.
4. Exibe um resumo completo ao usuário:
   - Arquivos alterados
   - Score da auditoria
   - Riscos identificados
   - Conteúdo do patch
5. Solicita confirmação explícita:
   ```txt
   Digite APPLY para aplicar o patch:
   ```
6. Somente após digitação exata de `APPLY`:
   ```bash
   git apply proposal.diff
   ```
7. Registra log da aplicação.

---

## Bloqueios automáticos

| Condição | Comportamento |
|---|---|
| Nenhum patch encontrado | Aborta com erro |
| Sem auditoria aprovada para o patch | Aborta. Execute `ai-orchestrator audit` primeiro |
| `git apply --check` falha | Aborta. O patch pode estar desatualizado |
| Confirmação não digitada exatamente como `APPLY` | Aborta sem aplicar |
| Segredo detectado no patch | Aborta imediatamente |

---

## Flags

| Flag | Descrição |
|---|---|
| `--patch <caminho>` | Especifica o patch a aplicar |
| `--dry-run` | Exibe o resumo e o comando que seria executado, sem aplicar |

---

## Confirmação obrigatória

O sistema exibe o seguinte prompt antes de qualquer aplicação:

```txt
══════════════════════════════════════
ATENÇÃO: Revisão final antes de aplicar
══════════════════════════════════════
Patch   : patches/proposal-001.diff
Arquivos: src/auth.ts, src/auth.test.ts
Score   : 92
Riscos  : Nenhum crítico
══════════════════════════════════════

Digite APPLY para aplicar o patch (qualquer outra entrada cancela):
```

Qualquer entrada que não seja exatamente `APPLY` (maiúsculas) cancela a operação.

---

## Após aplicação

```txt
✔ Patch aplicado com sucesso.
Arquivos alterados: src/auth.ts, src/auth.test.ts

Próximos passos sugeridos:
  git diff HEAD
  git add -p
  git commit -m "..."
```

---

## Arquivos gerados

| Arquivo | Conteúdo |
|---|---|
| `logs/apply-<timestamp>.json` | Log da aplicação com hash do patch e do commit gerado |

---

## Exemplo de log gerado

```json
{
  "timestamp": "2025-01-15T10:30:00Z",
  "patch_file": "patches/proposal-001.diff",
  "patch_hash": "sha256:abc123...",
  "audit_score": 92,
  "files_applied": ["src/auth.ts", "src/auth.test.ts"],
  "confirmed_by": "human",
  "result": "success"
}
```
