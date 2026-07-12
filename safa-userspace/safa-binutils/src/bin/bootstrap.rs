use std::{
    fs,
    process::{Command, Stdio},
};

fn main() {
    println!("Starting DHCP");
    if let Ok(net_dir) = fs::read_dir("dev:/net") {
        for ent in net_dir
            .filter_map(|ent| ent.ok())
            .filter(|e| e.file_type().is_ok_and(|f| f.is_file()))
        {
            let path = ent.path();
            println!("Configuring: {}", path.display());
            Command::new("sys:/bin/dhcpcli")
                .arg(path)
                .stdout(Stdio::inherit())
                .stdin(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
                .expect("Failed to spawn dhcpcli")
                .wait()
                .expect("Failed to wait for dhcpcli");
        }
    }

    println!("Starting AudioServer");
    Command::new("sys:/bin/luneaudio")
        .arg("-i")
        .stdout(Stdio::inherit())
        .stdin(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("Failed to spawn audioserver");
    println!("Starting UI");
    Command::new("sys:/bin/opal-wm")
        .arg("-i")
        .stdout(Stdio::inherit())
        .stdin(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("Failed to spawn UI");
}
