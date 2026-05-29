use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Result};
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};

pub struct PtySession {
    pub child_pid: u32,
    pub agent_name: String,
    writer: Box<dyn Write + Send>,
    output_buffer: Arc<Mutex<Vec<u8>>>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl PtySession {
    pub fn spawn(cli_cmd: &[&str], agent_name: &str) -> Result<Self> {
        if cli_cmd.is_empty() {
            return Err(anyhow!("O comando CLI não pode ser vazio"));
        }

        let pty_system = NativePtySystem::default();
        let pair = pty_system.openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let mut cmd = CommandBuilder::new(cli_cmd[0]);
        for arg in &cli_cmd[1..] {
            cmd.arg(arg);
        }

        let child = pair.slave.spawn_command(cmd)?;
        let child_pid = child.process_id().unwrap_or(0);
        
        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;

        let output_buffer = Arc::new(Mutex::new(Vec::new()));
        let buffer_clone = Arc::clone(&output_buffer);

        // Thread de leitura contínua: lê do PTY e joga no buffer
        // Encerra apenas quando o EOF for atingido (ex: processo pai ou CLI morre)
        thread::spawn(move || {
            let mut buf = [0u8; 1024];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 { break; }
                if let Ok(mut shared_buf) = buffer_clone.lock() {
                    shared_buf.extend_from_slice(&buf[..n]);
                }
            }
        });
        
        // A sessão não fecha automaticamente quando a struct é dropada.
        // A trait portable_pty::Child afirma: "Dropping the child does not kill the process".
        // Ele sobreviverá até o kill() explícito.
        Ok(Self {
            child_pid,
            agent_name: agent_name.to_string(),
            writer,
            output_buffer,
            child,
        })
    }

    pub fn send(&mut self, input: &str) -> Result<()> {
        self.writer.write_all(input.as_bytes())?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        Ok(())
    }

    pub fn read_output(&mut self, timeout_ms: u64) -> Result<String> {
        thread::sleep(Duration::from_millis(timeout_ms));
        
        let raw_output = {
            let mut buf = self.output_buffer.lock().map_err(|_| anyhow!("Falha ao travar o buffer do PTY"))?;
            let data = buf.clone();
            buf.clear();
            data
        };

        // CA: Limpeza rigorosa de escape codes ANSI emitidos por CLIs interativos
        let clean_bytes = strip_ansi_escapes::strip(&raw_output);
        Ok(String::from_utf8_lossy(&clean_bytes).to_string())
    }

    pub fn kill(&mut self) -> Result<()> {
        self.child.kill()?;
        Ok(())
    }
}
