//! Owned subprocess transport. Never invokes a shell. Windows children join a job before running.
use anyhow::{bail, Result};
use std::{
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, Command, Stdio},
};
use tokio::sync::mpsc;
const MAX_FRAME: usize = 8 * 1024 * 1024;
pub struct Process {
    child: Child,
    pub lines: mpsc::Receiver<Result<String>>,
    writer: mpsc::Sender<String>,
    #[cfg(windows)]
    job: Job,
}
impl Process {
    pub fn spawn(exe: &Path, args: &[String], cwd: &Path) -> Result<Self> {
        if !exe.is_absolute() || !exe.is_file() {
            bail!("Неверный executable")
        }
        #[cfg(windows)]
        if exe
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| !s.eq_ignore_ascii_case("exe"))
            .unwrap_or(true)
        {
            bail!("Требуется native .exe; shell wrappers не разрешены")
        }
        let mut cmd = Command::new(exe);
        cmd.args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Keep CLI authentication in its native store; do not inherit unrelated app secrets.
        cmd.env_clear();
        for key in [
            "PATH",
            "PATHEXT",
            "SYSTEMROOT",
            "WINDIR",
            "COMSPEC",
            "USERPROFILE",
            "HOME",
            "LOCALAPPDATA",
            "APPDATA",
            "TEMP",
            "TMP",
            "PROGRAMFILES",
            "PROGRAMFILES(X86)",
            "SYSTEMDRIVE",
            "CODEX_HOME",
            "CLAUDE_CONFIG_DIR",
        ] {
            if let Some(v) = std::env::var_os(key) {
                cmd.env(key, v);
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000 | 0x00000004);
        }
        let mut child = cmd.spawn()?;
        #[cfg(windows)]
        let job = match Job::attach_and_resume(&child) {
            Ok(j) => j,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("Missing stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("Missing stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow::anyhow!("Missing stderr"))?;
        let (tx, lines) = mpsc::channel(128);
        let (writer, mut rx) = mpsc::channel::<String>(32);
        std::thread::spawn(move || {
            while let Some(line) = rx.blocking_recv() {
                if stdin
                    .write_all(line.as_bytes())
                    .and_then(|_| stdin.write_all(b"\n"))
                    .and_then(|_| stdin.flush())
                    .is_err()
                {
                    break;
                }
            }
        });
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                match read_frame(&mut reader) {
                    Ok(Some(s)) => {
                        if tx.blocking_send(Ok(s)).is_err() {
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        let _ = tx.blocking_send(Err(e));
                        break;
                    }
                }
            }
        });
        // Drain stderr without persisting potentially sensitive CLI diagnostics.
        std::thread::spawn(move || {
            let mut r = BufReader::new(stderr);
            loop {
                match r.fill_buf() {
                    Ok(b) if !b.is_empty() => {
                        let n = b.len();
                        r.consume(n)
                    }
                    _ => break,
                }
            }
        });
        Ok(Self {
            child,
            lines,
            writer,
            #[cfg(windows)]
            job,
        })
    }
    pub async fn send(&mut self, value: serde_json::Value) -> Result<()> {
        let s = serde_json::to_string(&value)?;
        if s.len() > MAX_FRAME {
            bail!("Protocol input exceeds 8 MiB")
        };
        tokio::time::timeout(std::time::Duration::from_secs(10), self.writer.send(s)).await??;
        Ok(())
    }
    pub fn stop(&mut self) {
        #[cfg(windows)]
        self.job.kill();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        self.stop();
    }
}
fn read_frame(r: &mut impl BufRead) -> Result<Option<String>> {
    let mut out = Vec::new();
    loop {
        let b = r.fill_buf()?;
        if b.is_empty() {
            return if out.is_empty() {
                Ok(None)
            } else {
                Ok(Some(String::from_utf8(out)?))
            };
        }
        let n = b
            .iter()
            .position(|x| *x == b'\n')
            .map(|i| i + 1)
            .unwrap_or(b.len());
        if out.len() + n > MAX_FRAME {
            bail!("Protocol frame exceeds 8 MiB")
        };
        let done = b[n - 1] == b'\n';
        out.extend_from_slice(&b[..n]);
        r.consume(n);
        if done {
            return Ok(Some(String::from_utf8(out)?));
        }
    }
}

#[cfg(windows)]
struct Job(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
unsafe impl Send for Job {}
#[cfg(windows)]
impl Job {
    fn attach_and_resume(child: &Child) -> Result<Self> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::{
            Foundation::*,
            System::{Diagnostics::ToolHelp::*, JobObjects::*, Threading::*},
        };
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                bail!("CreateJobObject failed")
            };
            let job = Self(handle);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            ) == 0
                || AssignProcessToJobObject(handle, child.as_raw_handle()) == 0
            {
                bail!("Cannot contain CLI process in Job Object")
            }
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
            if snap == INVALID_HANDLE_VALUE {
                bail!("Cannot inspect suspended process")
            }
            let mut entry: THREADENTRY32 = std::mem::zeroed();
            entry.dwSize = std::mem::size_of_val(&entry) as u32;
            let mut ok = Thread32First(snap, &mut entry);
            let mut resumed = false;
            while ok != 0 {
                if entry.th32OwnerProcessID == child.id() {
                    let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                    if !thread.is_null() {
                        if ResumeThread(thread) != u32::MAX {
                            resumed = true
                        };
                        CloseHandle(thread);
                    }
                }
                ok = Thread32Next(snap, &mut entry);
            }
            CloseHandle(snap);
            if !resumed {
                bail!("Cannot resume contained CLI")
            };
            Ok(job)
        }
    }
    fn kill(&self) {
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.0, 1);
        }
    }
}
#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn framing() {
        let mut x = std::io::Cursor::new(b"one\ntwo\n");
        assert_eq!(read_frame(&mut x).unwrap(), Some("one\n".into()));
        assert_eq!(read_frame(&mut x).unwrap(), Some("two\n".into()));
        assert!(read_frame(&mut x).unwrap().is_none());
    }
    #[test]
    fn frame_bound() {
        assert!(read_frame(&mut std::io::Cursor::new(vec![b'x'; MAX_FRAME + 1])).is_err());
    }
}
