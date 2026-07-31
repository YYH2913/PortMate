use keyring_core::{Entry, Error};
use std::env;
use std::process::{Child, Command, ExitCode, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

const SERVICE: &str = "PortMate Native Keyring Probe";
const ACCOUNT_ENV: &str = "PORTMATE_NATIVE_KEYRING_PROBE_ACCOUNT";
const SECRET_ENV: &str = "PORTMATE_NATIVE_KEYRING_PROBE_SECRET";
const ROTATED_SECRET_ENV: &str = "PORTMATE_NATIVE_KEYRING_PROBE_ROTATED_SECRET";
const PHASE_TIMEOUT: Duration = Duration::from_secs(30);
const PROBE_SECRET_BYTES: usize = 1_200;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("native keyring probe failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut arguments = env::args().skip(1);
    let Some(argument) = arguments.next() else {
        return orchestrate();
    };
    if argument != "--phase" {
        return Err(format!(
            "unsupported native keyring probe argument: {argument}"
        ));
    }
    let phase = arguments
        .next()
        .ok_or_else(|| "native keyring probe phase is missing".to_string())?;
    if arguments.next().is_some() {
        return Err("native keyring probe accepts exactly one phase".to_string());
    }
    run_phase(&phase)
}

fn orchestrate() -> Result<(), String> {
    let account = format!("native-probe-{}", Uuid::new_v4());
    let secret = probe_secret("initial");
    let rotated_secret = probe_secret("rotated");
    let result = ["write", "verify-update", "verify-delete"]
        .into_iter()
        .try_for_each(|phase| run_child_phase(phase, &account, &secret, &rotated_secret));
    if result.is_err() {
        let _ = run_child_phase("cleanup", &account, &secret, &rotated_secret);
    }
    result?;
    println!(
        "PortMate native keyring probe passed on {} ({}-byte cross-process secret)",
        env::consts::OS,
        PROBE_SECRET_BYTES
    );
    Ok(())
}

fn run_child_phase(
    phase: &str,
    account: &str,
    secret: &str,
    rotated_secret: &str,
) -> Result<(), String> {
    let executable = env::current_exe()
        .map_err(|error| format!("resolve native keyring probe executable failed: {error}"))?;
    let mut child = Command::new(executable)
        .args(["--phase", phase])
        .env(ACCOUNT_ENV, account)
        .env(SECRET_ENV, secret)
        .env(ROTATED_SECRET_ENV, rotated_secret)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| format!("start native keyring probe phase {phase} failed: {error}"))?;
    wait_for_child(&mut child, phase)
}

fn wait_for_child(child: &mut Child, phase: &str) -> Result<(), String> {
    let deadline = Instant::now() + PHASE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                return Err(format!(
                    "native keyring probe phase {phase} exited with {status}"
                ));
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(100)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "native keyring probe phase {phase} exceeded its {}-second deadline",
                    PHASE_TIMEOUT.as_secs()
                ));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "wait for native keyring probe phase {phase} failed: {error}"
                ));
            }
        }
    }
}

fn run_phase(phase: &str) -> Result<(), String> {
    if phase == "expect-unavailable" {
        return match keyring::use_native_store(true) {
            Err(Error::PlatformFailure(_)) => Ok(()),
            Err(error) => Err(format!(
                "native keyring unavailable probe returned the wrong error: {error}"
            )),
            Ok(()) => Err("native keyring unexpectedly initialized without a provider".to_string()),
        };
    }

    keyring::use_native_store(true)
        .map_err(|error| format!("initialize persistent native keyring failed: {error}"))?;
    let account = required_environment(ACCOUNT_ENV)?;
    let secret = required_environment(SECRET_ENV)?;
    let rotated_secret = required_environment(ROTATED_SECRET_ENV)?;
    if secret.len() != PROBE_SECRET_BYTES || rotated_secret.len() != PROBE_SECRET_BYTES {
        return Err("native keyring probe secret length is invalid".to_string());
    }
    let entry = Entry::new(SERVICE, &account)
        .map_err(|error| format!("create native keyring probe entry failed: {error}"))?;

    match phase {
        "write" => {
            expect_missing(&entry, "before write")?;
            entry
                .set_password(&secret)
                .map_err(|error| format!("write native keyring probe secret failed: {error}"))
        }
        "verify-update" => {
            expect_password(&entry, &secret, "cross-process read")?;
            entry
                .set_password(&rotated_secret)
                .map_err(|error| format!("update native keyring probe secret failed: {error}"))?;
            expect_password(&entry, &rotated_secret, "updated read")
        }
        "verify-delete" => {
            expect_password(&entry, &rotated_secret, "pre-delete read")?;
            entry
                .delete_credential()
                .map_err(|error| format!("delete native keyring probe secret failed: {error}"))?;
            expect_missing(&entry, "after delete")?;
            match entry.delete_credential() {
                Ok(()) | Err(Error::NoEntry) => Ok(()),
                Err(error) => Err(format!(
                    "repeat native keyring probe delete failed: {error}"
                )),
            }
        }
        "verify-locked" => match entry.get_password() {
            Err(Error::NoStorageAccess(_)) => Ok(()),
            #[cfg(target_os = "macos")]
            Err(Error::PlatformFailure(_)) => Ok(()),
            Err(error) => Err(format!(
                "native keyring locked probe returned the wrong error: {error}"
            )),
            Ok(_) => Err("native keyring unexpectedly read a locked credential".to_string()),
        },
        "cleanup" => match entry.delete_credential() {
            Ok(()) | Err(Error::NoEntry) => Ok(()),
            Err(error) => Err(format!("native keyring probe cleanup failed: {error}")),
        },
        _ => Err(format!("unsupported native keyring probe phase: {phase}")),
    }
}

fn expect_password(entry: &Entry, expected: &str, label: &str) -> Result<(), String> {
    let actual = entry
        .get_password()
        .map_err(|error| format!("native keyring probe {label} failed: {error}"))?;
    if actual != expected {
        return Err(format!("native keyring probe {label} content mismatch"));
    }
    Ok(())
}

fn expect_missing(entry: &Entry, label: &str) -> Result<(), String> {
    match entry.get_password() {
        Err(Error::NoEntry) => Ok(()),
        Ok(_) => Err(format!(
            "native keyring probe entry unexpectedly exists {label}"
        )),
        Err(error) => Err(format!(
            "native keyring probe missing-entry check {label} failed: {error}"
        )),
    }
}

fn required_environment(name: &str) -> Result<String, String> {
    env::var(name)
        .ok()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("native keyring probe environment variable {name} is missing"))
}

fn probe_secret(label: &str) -> String {
    let mut secret = format!("portmate-native-keyring-{label}-{}", Uuid::new_v4());
    secret.extend(std::iter::repeat_n('x', PROBE_SECRET_BYTES - secret.len()));
    secret
}
