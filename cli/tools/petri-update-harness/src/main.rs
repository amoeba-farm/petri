#![allow(dead_code)]
#[path = "../../../src/release_update/mod.rs"]
mod release_update;

fn main() {
    if let Some(result) = release_update::helper_entry() {
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).is_some_and(|s| s == "--version") {
        println!("petri 0.1.3");
        return;
    }
    // A real, locked parent executable for the Windows-only local fixture check.
    // This driver mutates only the caller-created private test fixture.
    if args.len() == 3 && args[1] == "--fixture-parent" {
        let stage = std::path::Path::new(&args[2])
            .canonicalize()
            .expect("fixture stage");
        assert!(
            stage
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".petri-update-")
        );
        let mut plan: serde_json::Value =
            serde_json::from_slice(&std::fs::read(stage.join("plan.json")).unwrap()).unwrap();
        plan["parentPid"] = std::process::id().into();
        std::fs::write(stage.join("plan.json"), serde_json::to_vec(&plan).unwrap()).unwrap();
        let helper = stage.join(if cfg!(windows) {
            "petri-update-helper.exe"
        } else {
            "petri-update-helper"
        });
        let mut command = std::process::Command::new(helper);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        command.arg("--petri-apply-update").spawn().unwrap();
        let start = std::time::Instant::now();
        while !stage.join("ready").is_file() {
            assert!(start.elapsed().as_secs() < 30, "helper handshake timed out");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        println!("helper-ready-parent-still-running");
        std::thread::sleep(std::time::Duration::from_secs(2));
        return;
    }
    println!("{}", release_update::information());
}
