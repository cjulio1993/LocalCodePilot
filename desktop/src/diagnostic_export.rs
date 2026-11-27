//! "Exportar diagnóstico" (T034): a JSON file a person can send to support. Only the fields
//! listed in [`Report`] go into it, never logs, `.env` or manifest contents, and every text
//! passes through [`Sanitizer`] before it is written: secrets are masked, the project folder
//! and the personal folder are replaced by labels, and a value that still looks sensitive is
//! left out. The format is documented in docs/DETALHES-TECNICOS.md.

use crate::secrets::{arguments_look_secret, mask_secrets};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// The name and version of the file format.
pub const FORMAT: &str = "localcodepilot-diagnostico";
pub const FORMAT_VERSION: u32 = 1;
/// What a fact the app did not check says.
pub const NOT_VERIFIED: &str = "não verificado";
/// What a value left out says.
pub const OMITTED: &str = "omitido: pode conter dado sensível";

/// What the file says about a project, gathered by the app.
pub struct Report<'a> {
    pub project_name: &'a str,
    pub project_path: &'a Path,
    pub technologies: Vec<String>,
    pub state: &'a str,
    pub reason: Option<&'a str>,
    pub rule: &'a str,
    pub evidence: &'a [String],
    pub technical: &'a [String],
    pub commands: Vec<Command>,
    pub requirements: Vec<String>,
    pub databases: Vec<Database>,
}

/// A command the app found for the project.
pub struct Command {
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
    pub folder: PathBuf,
}

/// A database the project uses: never a user, a password or an address of another computer.
pub struct Database {
    pub engine: String,
    pub port: Option<u16>,
    /// The server is on this computer; `None`: not verified.
    pub local: Option<bool>,
    /// The server answered; `None`: not verified.
    pub running: Option<bool>,
}

/// The file's text: pretty JSON.
pub fn render(report: &Report<'_>, home: Option<&Path>) -> String {
    let sanitizer = Sanitizer::new(report.project_path, home);
    let text = |value: &str| sanitizer.text(value).unwrap_or_else(|| OMITTED.to_owned());
    let lines = |values: &[String]| values.iter().map(|value| text(value)).collect::<Vec<_>>();
    let document = json!({
        "formato": FORMAT,
        "versao_formato": FORMAT_VERSION,
        "app": {
            "versao": env!("CARGO_PKG_VERSION"),
            "sistema": std::env::consts::OS,
            "arquitetura": std::env::consts::ARCH,
        },
        "projeto": {
            "nome": text(report.project_name),
            "identificador": project_identifier(report.project_path),
            "tecnologias": lines(&report.technologies),
        },
        "diagnostico": {
            "estado": report.state,
            "motivo": report.reason.map_or_else(|| Value::Null, |reason| Value::String(text(reason))),
            "regra": report.rule,
            "encontrado": if report.evidence.is_empty() {
                vec![NOT_VERIFIED.to_owned()]
            } else {
                lines(report.evidence)
            },
            "detalhes_tecnicos": lines(report.technical),
        },
        "comandos": report.commands.iter().map(|command| json!({
            "nome": text(&command.name),
            "comando": sanitizer
                .command(&command.program, &command.args)
                .unwrap_or_else(|| OMITTED.to_owned()),
            "pasta": text(&command.folder.to_string_lossy()),
        })).collect::<Vec<_>>(),
        "requisitos": lines(&report.requirements),
        "bancos": report.databases.iter().map(|database| json!({
            "motor": text(&database.engine),
            "porta": database.port.map_or_else(|| Value::String(NOT_VERIFIED.into()), Value::from),
            "servidor": match database.local {
                Some(true) => "este computador",
                Some(false) => "outro computador",
                None => NOT_VERIFIED,
            },
            "responde": database.running.map_or_else(|| Value::String(NOT_VERIFIED.into()), Value::from),
        })).collect::<Vec<_>>(),
    });
    serde_json::to_string_pretty(&document).unwrap_or_default()
}

/// A stable identifier that does not reveal the folder: SHA-256 of the folder, 12 characters.
pub fn project_identifier(path: &Path) -> String {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut text = path.to_string_lossy().into_owned();
    if cfg!(windows) {
        text = text.to_lowercase();
    }
    Sha256::digest(text.as_bytes())
        .iter()
        .take(6)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A file name for the export, without characters a file system refuses.
pub fn file_name(project_name: &str) -> String {
    let name = project_name
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    format!("diagnostico-{}.json", name.trim_matches('-'))
}

/// Cleans every text that goes into the file.
pub struct Sanitizer {
    /// Folders replaced by a label, longest first: the project folder sits inside the
    /// personal folder.
    folders: Vec<(String, &'static str)>,
}

impl Sanitizer {
    pub fn new(project_path: &Path, home: Option<&Path>) -> Self {
        let mut folders = Vec::new();
        let mut add = |path: &Path, label: &'static str| {
            // Without the `\\?\` of canonical Windows paths: the texts use the plain form.
            let full = path.to_string_lossy();
            let text = full
                .strip_prefix(r"\\?\")
                .unwrap_or(&full)
                .trim_end_matches(['/', '\\'])
                .to_owned();
            if text.len() > 3 {
                folders.push((text.replace('\\', "/"), label));
                folders.push((text.replace('/', "\\"), label));
            }
        };
        add(project_path, "<projeto>");
        if let Ok(canonical) = std::fs::canonicalize(project_path) {
            add(&canonical, "<projeto>");
        }
        if let Some(home) = home {
            add(home, "<pasta pessoal>");
        }
        folders.sort_by_key(|(folder, _)| std::cmp::Reverse(folder.len()));
        Self { folders }
    }

    /// The text without secrets or personal folders; `None` when it still looks sensitive.
    pub fn text(&self, text: &str) -> Option<String> {
        let mut clean = text.strip_prefix(r"\\?\").unwrap_or(text).to_owned();
        for (folder, label) in &self.folders {
            clean = replace_ignoring_case(&clean, folder, label);
        }
        clean = personal_folders(&clean);
        clean = addresses(&clean);
        clean = mask_secrets(&clean);
        clean = mask_connection_passwords(&clean);
        clean = mask_option_values(&clean);
        (!looks_sensitive(&clean)).then_some(clean)
    }

    /// A command line, left out whole when an argument looks like a secret.
    pub fn command(&self, program: &str, args: &[String]) -> Option<String> {
        if arguments_look_secret(args) {
            return None;
        }
        let line = std::iter::once(program.to_owned())
            .chain(args.iter().cloned())
            .collect::<Vec<_>>()
            .join(" ");
        self.text(&line)
    }
}

/// In an address (`scheme://user:password@host:port/...`), the user and the password are
/// left out, and a server other than this computer becomes `<servidor>`: its name may say
/// something about the infrastructure.
fn addresses(text: &str) -> String {
    let mut result = text.to_owned();
    let mut from = 0;
    while let Some(found) = result[from..].find("://") {
        let start = from + found + "://".len();
        let end = result[start..]
            .find(['/', ' ', '"', '\'', ';', ',', ')'])
            .map_or(result.len(), |offset| start + offset);
        let authority = &result[start..end];
        let host_port = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        let host = host_port
            .rsplit_once(':')
            .filter(|(_, port)| port.chars().all(|character| character.is_ascii_digit()))
            .map_or(host_port, |(host, _)| host);
        let port = &host_port[host.len()..];
        let local = matches!(
            host.to_ascii_lowercase().as_str(),
            "localhost" | "127.0.0.1" | "[::1]" | "0.0.0.0" | ""
        );
        let clean = if local {
            host_port.to_owned()
        } else {
            format!("<servidor>{port}")
        };
        result.replace_range(start..end, &clean);
        from = start + clean.len();
        if from >= result.len() {
            break;
        }
    }
    result
}

/// `C:\Users\ana`, `/Users/ana` and `/home/ana` become `<usuário>`, whatever follows.
fn personal_folders(text: &str) -> String {
    let mut result = text.to_owned();
    for prefix in ["\\users\\", "/users/", "/home/"] {
        let mut from = 0;
        // ASCII lowercase keeps byte offsets, so the positions apply to the original text.
        while let Some(found) = result.to_ascii_lowercase()[from..].find(prefix) {
            let start = from + found + prefix.len();
            let end = result[start..]
                .find(['/', '\\', '"', '\'', ' ', ';', ','])
                .map_or(result.len(), |offset| start + offset);
            if start < end && !result[start..].starts_with('<') {
                result.replace_range(start..end, "<usuário>");
            }
            from = start;
            if from >= result.len() {
                break;
            }
        }
    }
    result
}

/// `Password=x;` is masked by [`mask_secrets`]; the password (`Pwd=x;`) and the user
/// (`User Id=sa;`, `Uid=sa;`) of connection strings are not.
fn mask_connection_passwords(text: &str) -> String {
    let mut result = text.to_owned();
    for key in ["pwd=", "uid=", "user id=", "user="] {
        let mut from = 0;
        while let Some(found) = result.to_ascii_lowercase()[from..].find(key) {
            let start = from + found + key.len();
            let end = result[start..]
                .find([';', ' ', '"', '\''])
                .map_or(result.len(), |offset| start + offset);
            if start < end {
                result.replace_range(start..end, crate::secrets::MASK);
            }
            from = start;
            if from >= result.len() {
                break;
            }
        }
    }
    result
}

/// In free text, the value after an option named like a secret (`--token abc`) is masked,
/// as `arguments_look_secret` does for the arguments of a command.
fn mask_option_values(text: &str) -> String {
    let words = text.split(' ').collect::<Vec<_>>();
    let mut changed = false;
    let mut result = Vec::with_capacity(words.len());
    let mut mask_next = false;
    for (index, word) in words.iter().enumerate() {
        if mask_next && !word.is_empty() {
            result.push(crate::secrets::MASK.to_owned());
            mask_next = false;
            changed = true;
            continue;
        }
        result.push((*word).to_owned());
        if let Some(next) = words.get(index + 1)
            && word.starts_with('-')
            && arguments_look_secret(&[(*word).to_owned(), (*next).to_owned()])
        {
            mask_next = true;
        }
    }
    if changed {
        result.join(" ")
    } else {
        text.to_owned()
    }
}

/// What the masks above may have missed: credentials in an address (`user:pass@host` without
/// a scheme the masks know) or a private key.
fn looks_sensitive(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("private key")
        || lower.contains("-----begin")
        || text.split_whitespace().any(|word| {
            let word = word.trim_matches(['"', '\'', ',', ';', '(', ')']);
            word.contains('@')
                && word.contains(':')
                && !word.contains(crate::secrets::MASK)
                && word.find(':') < word.find('@')
        })
}

fn replace_ignoring_case(text: &str, needle: &str, replacement: &str) -> String {
    if needle.is_empty() {
        return text.to_owned();
    }
    let lower_text = text.to_lowercase();
    let lower_needle = needle.to_lowercase();
    // Lowercasing may change byte lengths outside ASCII; then only exact matches are replaced.
    if lower_text.len() != text.len() || lower_needle.len() != needle.len() {
        return text.replace(needle, replacement);
    }
    let mut result = String::with_capacity(text.len());
    let mut cursor = 0;
    while let Some(found) = lower_text[cursor..].find(&lower_needle) {
        let start = cursor + found;
        result.push_str(&text[cursor..start]);
        result.push_str(replacement);
        cursor = start + needle.len();
    }
    result.push_str(&text[cursor..]);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unique fake secrets, each placed in a different source and format. None may reach the
    /// file.
    const SECRETS: [&str; 12] = [
        "segredo-env-7f3a",
        "segredo-json-91bc",
        "segredo-bearer-55de",
        "segredo-opcao-a1b2",
        "segredo-url-c3d4",
        "segredo-conexao-e5f6",
        "segredo-pwd-0a0b",
        "segredo-token-9c9d",
        "segredo-chave-8e8f",
        "anaprivada",
        "bruno.silva",
        "carla_m",
    ];

    fn report_with_secrets(project: &Path, home: &Path) -> String {
        let evidence = vec![
            "DB_PASSWORD=segredo-env-7f3a".to_owned(),
            r#"{"password": "segredo-json-91bc"}"#.to_owned(),
            "Authorization: Bearer segredo-bearer-55de".to_owned(),
            "falha em postgres://admin:segredo-url-c3d4@db.interno:5432/loja".to_owned(),
            "Server=db;User Id=sa;Password=segredo-conexao-e5f6;".to_owned(),
            "Data Source=db;Pwd=segredo-pwd-0a0b;".to_owned(),
            format!("arquivo {}", project.join(".env").display()),
            r"lido em C:\Users\anaprivada\Documents\x.txt".to_owned(),
            "lido em /Users/bruno.silva/projetos/y".to_owned(),
            "lido em /home/carla_m/z".to_owned(),
            "API_KEY: segredo-chave-8e8f".to_owned(),
        ];
        let technical = vec![
            "Comando: npm run dev --token segredo-token-9c9d".to_owned(),
            format!("Pasta procurada: {}", home.join("outra").display()),
        ];
        let report = Report {
            project_name: "loja",
            project_path: project,
            technologies: vec!["Node".into()],
            state: "Com problema",
            reason: Some("web parou com erro. Veja os logs para encontrar a causa."),
            rule: "Processo parou com erro",
            evidence: &evidence,
            technical: &technical,
            commands: vec![
                Command {
                    name: "dev".into(),
                    program: "npm".into(),
                    args: vec![
                        "run".into(),
                        "dev".into(),
                        "--token".into(),
                        "segredo-opcao-a1b2".into(),
                    ],
                    folder: project.to_path_buf(),
                },
                Command {
                    name: "web".into(),
                    program: home.join("bin").join("node").to_string_lossy().into_owned(),
                    args: vec!["server.js".into()],
                    folder: project.join("web"),
                },
            ],
            requirements: vec!["Node 22: não encontrado.".into()],
            databases: vec![Database {
                engine: "PostgreSQL".into(),
                port: Some(5432),
                local: Some(false),
                running: None,
            }],
        };
        render(&report, Some(home))
    }

    #[test]
    fn no_fake_secret_or_personal_folder_reaches_the_file() {
        for (home, project) in [
            (r"C:\Users\anaprivada", r"C:\Users\anaprivada\projetos\loja"),
            ("/Users/bruno.silva", "/Users/bruno.silva/projetos/loja"),
            ("/home/carla_m", "/home/carla_m/projetos/loja"),
        ] {
            let file = report_with_secrets(Path::new(project), Path::new(home));
            for secret in SECRETS {
                assert!(
                    !file.to_lowercase().contains(&secret.to_lowercase()),
                    "{secret} vazou:\n{file}"
                );
            }
            assert!(!file.contains("db.interno"), "{file}");
            assert!(!file.contains("admin"), "{file}");
            assert!(file.contains("postgres://<servidor>:5432/loja"), "{file}");
            // What stays readable for support.
            assert!(
                file.contains("\"regra\": \"Processo parou com erro\""),
                "{file}"
            );
            assert!(file.contains("<projeto>"), "{file}");
            assert!(
                file.contains("\"servidor\": \"outro computador\""),
                "{file}"
            );
            assert!(file.contains("\"responde\": \"não verificado\""), "{file}");
            assert!(file.contains(OMITTED), "{file}");
            let parsed: Value = serde_json::from_str(&file).unwrap();
            assert_eq!(parsed["formato"], FORMAT);
            assert_eq!(parsed["versao_formato"], FORMAT_VERSION);
            assert_eq!(
                parsed["projeto"]["identificador"].as_str().unwrap().len(),
                12
            );
        }
    }

    #[test]
    fn a_canonical_windows_project_path_is_still_the_project() {
        let sanitizer = Sanitizer::new(Path::new(r"\\?\C:\dev\clientes\loja"), None);
        assert_eq!(
            sanitizer.text(r"C:\dev\clientes\loja\web").as_deref(),
            Some(r"<projeto>\web")
        );
        assert_eq!(
            sanitizer.text(r"\\?\C:\dev\clientes\loja").as_deref(),
            Some("<projeto>")
        );
    }

    #[test]
    fn missing_facts_are_not_verified_and_the_file_name_is_safe() {
        let report = Report {
            project_name: "loja: nova/versão",
            project_path: Path::new("/tmp/loja"),
            technologies: vec![],
            state: "Parado",
            reason: None,
            rule: "Pronto para iniciar",
            evidence: &[],
            technical: &[],
            commands: vec![],
            requirements: vec![],
            databases: vec![Database {
                engine: "MySQL".into(),
                port: None,
                local: None,
                running: None,
            }],
        };
        let parsed: Value = serde_json::from_str(&render(&report, None)).unwrap();
        assert_eq!(parsed["diagnostico"]["encontrado"][0], NOT_VERIFIED);
        assert_eq!(parsed["diagnostico"]["motivo"], Value::Null);
        assert_eq!(parsed["bancos"][0]["porta"], NOT_VERIFIED);
        assert_eq!(parsed["bancos"][0]["servidor"], NOT_VERIFIED);
        assert_eq!(
            file_name("loja: nova/versão"),
            "diagnostico-loja--nova-versão.json"
        );
    }
}
