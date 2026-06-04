# TODO Retomada V1 — ai-orquestrator

Data: 2026-06-03

## Regra de contexto

- [x] Consultar e salvar ai-memory pelo projeto resolvido a partir do `cwd`, sem `--project` fixo no binário instalado.
- [x] Exibir o projeto ativo no TUI quando houver integração com ai-memory.
- [x] Remover escopo fixo `--project ai-orquestrator` das chamadas locais ao CLI `ai-memory`.

## Retomada dos handoffs antigos

- [x] Ler e consolidar `rules/ai-memory-usage.md`.
- [x] Ler e consolidar `notes/session_summary_tui_pivot.md`.
- [x] Ler e consolidar `notes/ratatui-fase-5.3.md`.
- [x] Validar estado de `notes/tui-fase-5.7-first-run-setup.md` — implementado; teste manual ainda pendente.
- [x] Validar estado de `notes/tui-fase-5.8-memory-view.md` — implementado; teste manual ainda pendente.

## Estabilização antes de novas features

- [x] Revalidar worktree atual e separar mudanças reais de ruído gerado (`git status -- ':!target'`).
- [x] Corrigir `.gitignore` para evitar `target/` e artefatos de build.
- [x] Decidir destino dos diffs soltos `step*.diff` — tratados como artefatos locais via `.gitignore`.
- [x] Limpar warnings de produção reportados por `cargo check`.
- [x] Rodar `cargo test`.
- [x] Rodar `cargo clippy --all-targets -- -D warnings`.

## Invariantes de segurança e auditoria

- [x] Remover aprovação automática/fake no ciclo Dev -> Auditor.
- [x] Fazer `execute_tui` usar `cli_audit` de verdade.
- [x] Tornar violação de segredo hard-stop, sem bypass por gate.
- [x] Centralizar aplicação de patch no fluxo rigoroso de `apply`.
- [x] Garantir que nenhum patch aplique sem auditoria real, hash, `git apply --check` e confirmação humana.

## TUI atual

- [ ] Testar manualmente first-run setup.
- [ ] Testar manualmente AI Memory View.
- [x] Garantir logs em tempo real sem depender de keypress — validado em teste manual; roteamento dos panes ajustado.
- [x] Priorizar enriquecimento no gate de planejamento antes de continuar para o próximo turno.
- [x] Exigir briefing humano inicial antes do primeiro turno do Arquiteto.
- [x] Impedir IA Dev de implementação no planejamento; segundo CLI agora é Revisor de Planejamento.
- [x] Adicionar foco, scroll independente e cópia por painel nos panes de agentes.
- [x] Corrigir scroll dos panes para usar buffer completo em vez de fatiar texto antes do render.
- [x] Tornar envio ao Revisor ação explícita (`R`) no gate de planejamento, sem alternância automática.
- [x] Remover duplicação de conteúdo no gate e impedir reclassificação de painel por palavras dentro da resposta.
- [x] Remover ação "continuar para Arquiteto" do gate de planejamento; `C/F` agora congela o plano e `Esc` não aborta.
- [x] Usar painéis em pilha por padrão, com atalho para alternar lado a lado.
- [x] Capturar `stderr` dos CLIs para impedir warnings externos de escreverem por cima do TUI.
- [x] Rodar Gemini CLI em modo headless confiável (`GEMINI_CLI_TRUST_WORKSPACE=true`).
- [x] Remover painel de Auditora do planejamento; Auditora aparece apenas no ciclo Dev.
- [ ] Garantir que tela de erro permanece legível até ação do usuário.
- [x] Remover texto "vazio = aprovação automática" dos prompts de auditora.

## Próxima evolução: painéis de agentes

- [x] Definir abstração `AgentTerminal`.
- [ ] Projetar backend `pty-native` para painéis Ratatui reais.
- [ ] Projetar backend `tmux` opcional controlado pelo TUI.
- [x] Fazer spike pequeno: topo com gate humano, esquerda Dev, direita Auditor.
- [ ] Manter execução serial por turno no planejamento até decisão contrária.
- [ ] Persistir saídas estruturadas no SQLite; visualização não substitui validação.

## Critério para seguir

- [x] Todos os itens críticos de segurança/auditoria fechados antes de implementar painéis multi-agente como feature principal.
