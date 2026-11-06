#[path = "build_support.rs"]
mod build_support;

use std::{env, fs, path::PathBuf};

const ICON: &str = "../assets/branding/localcodepilot.ico";

fn main() {
    println!("cargo:rerun-if-changed=app.rc.in");
    println!("cargo:rerun-if-changed=build_support.rs");
    println!("cargo:rerun-if-changed={ICON}");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    // The version fields come from the package version, so a release can never
    // ship with the version of an older one (#99). Cargo reruns this script
    // whenever CARGO_PKG_VERSION changes.
    let version = env::var("CARGO_PKG_VERSION").expect("Cargo sets CARGO_PKG_VERSION");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("Cargo sets OUT_DIR"));
    // The icon sits next to the generated script, so the script names it without
    // a path: the repository folder may contain spaces or accents.
    fs::copy(ICON, out_dir.join("localcodepilot.ico")).expect("copy the Windows icon");
    let template = fs::read_to_string("app.rc.in").expect("read app.rc.in");
    let resource = build_support::render_resource(&template, &version, "localcodepilot.ico")
        .unwrap_or_else(|error| panic!("{error}"));
    let script = out_dir.join("app.rc");
    fs::write(&script, resource).expect("write the Windows resource script");
    embed_resource::compile(&script, embed_resource::NONE)
        .manifest_optional()
        .expect("the LocalCodePilot Windows resource must compile");
}
