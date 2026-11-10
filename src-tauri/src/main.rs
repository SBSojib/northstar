fn main() {
    let askpass = std::env::var("NORTHSTAR_ASKPASS").ok().as_deref() == Some("1")
        && std::env::var_os("NORTHSTAR_ASKPASS_DIR").is_some();
    if askpass {
        northstar_ssh_lib::askpass::main();
    }
    northstar_ssh_lib::run();
}
