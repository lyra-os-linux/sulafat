use std::path::Path;
use std::process::Command;

const DOMAIN: &str = "sulafat";

/// Compila `po/*.po` em `po/locale/<locale>/LC_MESSAGES/sulafat.mo`, de onde
/// o binário de desenvolvimento lê as traduções (o RPM instala as próprias).
fn main() {
    println!("cargo:rerun-if-changed=../po");
    let po_dir = Path::new("../po");
    assert!(
        Command::new("msgfmt").arg("--version").output().is_ok(),
        "msgfmt is required to build Sulafat translation catalogs"
    );
    let linguas = std::fs::read_to_string(po_dir.join("LINGUAS")).expect("read po/LINGUAS");
    for locale in linguas.split_whitespace() {
        let source = po_dir.join(format!("{locale}.po"));
        let output_dir = po_dir.join("locale").join(locale).join("LC_MESSAGES");
        std::fs::create_dir_all(&output_dir).expect("create local catalog directory");
        let status = Command::new("msgfmt")
            .arg("--check")
            .arg("--check-format")
            .arg("-o")
            .arg(output_dir.join(format!("{DOMAIN}.mo")))
            .arg(&source)
            .status()
            .expect("run msgfmt");
        assert!(status.success(), "failed to compile {}", source.display());
    }
    tauri_build::build();
}
