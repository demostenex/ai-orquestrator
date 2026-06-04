# Project: ai-orquestrator

Este é o AI Orchestrator, um sistema multi-agente em Rust projetado para coordenar IAs externas (CLIs) sobre um codebase de forma segura e auditável.

## Regras de Arquitetura
- Linguagem: Rust
- Persistência: SQLite (Single Source of Truth)
- Interface: CLI + TUI (inquire)
- Auditoria: Rigorosa, com commits atômicos e notas de implementação no ai-memory.

## Memória Local
- O projeto está na fase de implementação do Passo 5 (Modo Dev orientado a tarefas).
- Nenhuma IA aplica código diretamente sem aprovação humana e auditoria.
