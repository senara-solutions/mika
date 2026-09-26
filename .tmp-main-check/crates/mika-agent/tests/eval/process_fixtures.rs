//! Fixtures de processus réels, partagées par les tests eval (mika#2265, AC2).
//!
//! Ces trois helpers vivaient dans `test_pilot_silent_stall_reaper.rs` comme
//! `fn` privées de module. Ils sont ici parce que la connaissance qu'ils portent
//! — pourquoi `process_group(0)`, et pourquoi un enfant non *reapé* ment — doit
//! voyager avec le helper plutôt que rester dans un fichier. Le prochain test qui
//! assert un cycle de vie de processus (le fix reaper de mika#2272, entre autres)
//! a besoin des deux paragraphes autant que du code.
//!
//! Les deux formes ne sont pas interchangeables :
//! [`spawn_live_child`] laisse un enfant **vivant**, [`spawn_and_reap_child`]
//! rend un PID dont le processus est **réellement parti**. Prendre l'un pour
//! l'autre fait asserter au test le contraire de ce qu'il prétend.

use std::process::Command;

/// A real, killable child. `sleep 600` outlives every test that uses it, so a
/// case that asserts "not reaped" is asserting on a process that is genuinely
/// still alive rather than on a race.
///
/// Two details of this fixture are load-bearing, and both exist to make it
/// match production rather than to make the test pass.
///
/// **`process_group(0)`** — the executor spawns every dispatch as a process
/// group leader (`skills/executor.rs`), and `kill_process_gracefully` signals
/// the **group** first. That first attempt reports success whether or not a
/// matching group exists: `/bin/kill -TERM -<n>` exits 0 on this platform even
/// when no such group is there, so the single-PID fallback behind the `||` is
/// never reached. A child spawned without a group of its own would therefore
/// receive no signal at all, the kill would report failure, the reaper would
/// (correctly) decline to transition a task whose process it could not dispose
/// of — and the test would be measuring its own fixture rather than the code.
///
/// **The reaping thread** — `is_process_alive` tests `/proc/<pid>/stat`, which
/// a **zombie** still has. In production the executor's `tokio::spawn` awaits
/// the child, so the zombie is reaped the instant it dies. This thread
/// reproduces that, and nothing more.
pub fn spawn_live_child() -> (i64, u64) {
    use std::os::unix::process::CommandExt;
    let mut child = Command::new("sleep")
        .arg("600")
        .process_group(0)
        .spawn()
        .expect("spawn sleep");
    let pid = i64::from(child.id());
    let start_time = mika_agent::task_engine::process_liveness::read_process_start_time(child.id())
        .expect("read child start time");
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    (pid, start_time)
}

pub fn kill_pid(pid: i64) {
    let _ = Command::new("kill").arg("-9").arg(pid.to_string()).output();
}

/// A PID whose process is genuinely **gone** — killed *and reaped*.
///
/// [`spawn_live_child`] leaks its handle, which is right for the live cases but
/// wrong here: an unreaped child becomes a zombie, `/proc/<pid>/stat` survives
/// with its original start time, and `is_same_process_alive` correctly answers
/// *alive*. A test built on a zombie would assert the opposite of what it
/// claims. So this one waits on the child before returning.
pub fn spawn_and_reap_child() -> (i64, u64) {
    use std::os::unix::process::CommandExt;
    let mut child = Command::new("sleep")
        .arg("600")
        .process_group(0)
        .spawn()
        .expect("spawn sleep");
    let pid = i64::from(child.id());
    let start_time = mika_agent::task_engine::process_liveness::read_process_start_time(child.id())
        .expect("read child start time");
    child.kill().expect("kill child");
    child.wait().expect("reap child");
    (pid, start_time)
}
