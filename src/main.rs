//! Command line: `sheeter run` for one file, `sheeter watch` for a folder.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use sheeter::{Error, Job};

const HELP: &str = "\
sheeter: impose PDF pages onto press-ready sheets

USAGE
  sheeter run <input.pdf> [options]
  sheeter watch --job job.toml --in <dir> --out <dir> [--every <secs>] [--once]

OPTIONS
  -o, --out <path>      where to write (default: <input>-sheets.pdf)
      --job <file>      job description in TOML
      --sheet <size>    A4, A3L, SRA3, 320x450, 12x18in
      --layout <kind>   nup, booklet, step
      --cols <n>        tiles across
      --rows <n>        tiles down
      --margin <len>    clear edge, default 0
      --gutter <len>    space between tiles, default 0
      --bleed <len>     content overhang past trim, default 0
      --creep <len>     per-sheet shift for booklets, default 0
      --marks <list>    crop, reg or none, comma separated
      --fit <mode>      fit (keep aspect) or fill (stretch)
      --copies <n>      sheets a step job produces
      --pages <list>    which pages to use, 1 based: 1-8,12
      --password <pw>   password for a protected input
      --report <file>   write the placement map as CSV
      --duplex          two sides per sheet
      --mirror-back     mirror back sides for long-edge flipping
  -h, --help
  -V, --version

A bare length is millimetres. Suffixes: mm, cm, in, pt.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match dispatch(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("sheeter: {err}");
            ExitCode::from(2)
        }
    }
}

fn dispatch(args: &[String]) -> Result<(), Error> {
    let Some(first) = args.first() else {
        print!("{HELP}");
        return Ok(());
    };
    if first == "-h" || first == "--help" || first == "help" {
        print!("{HELP}");
        return Ok(());
    }
    if first == "-V" || first == "--version" {
        println!("sheeter {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let rest = &args[1..];
    match first.as_str() {
        "run" | "impose" => run_command(rest),
        "watch" => watch_command(rest),
        other => Err(Error::Config(format!(
            "{other:?} is not a command: use run or watch"
        ))),
    }
}

#[derive(Default)]
struct Flags {
    values: HashMap<String, String>,
    switches: Vec<String>,
    positional: Vec<String>,
}

impl Flags {
    fn parse(args: &[String], with_values: &[&str], switches: &[&str]) -> Result<Self, Error> {
        let mut flags = Self::default();
        let mut index = 0;
        while index < args.len() {
            let arg = &args[index];
            if arg == "--" {
                flags.positional.extend(args[index + 1..].iter().cloned());
                break;
            }
            if let Some(name) = arg.strip_prefix("--") {
                let (key, inline) = match name.split_once('=') {
                    Some((key, value)) => (key, Some(value.to_string())),
                    None => (name, None),
                };
                if switches.contains(&key) {
                    if inline.is_some() {
                        return Err(Error::Config(format!("--{key} takes no value")));
                    }
                    flags.switches.push(key.to_string());
                } else if with_values.contains(&key) {
                    let value = match inline {
                        Some(value) => value,
                        None => {
                            index += 1;
                            args.get(index)
                                .cloned()
                                .ok_or_else(|| Error::Config(format!("--{key} needs a value")))?
                        }
                    };
                    flags.values.insert(key.to_string(), value);
                } else {
                    return Err(Error::Config(format!("unknown option --{key}")));
                }
            } else if arg.len() > 1 && arg.starts_with('-') {
                let key = &arg[1..];
                let canonical = match key {
                    "o" => "out",
                    "h" => "help",
                    "V" => "version",
                    other => other,
                };
                if canonical == "help" || canonical == "version" {
                    flags.switches.push(canonical.to_string());
                } else if with_values.contains(&canonical) {
                    index += 1;
                    let value = args
                        .get(index)
                        .cloned()
                        .ok_or_else(|| Error::Config(format!("-{key} needs a value")))?;
                    flags.values.insert(canonical.to_string(), value);
                } else {
                    return Err(Error::Config(format!("unknown option -{key}")));
                }
            } else {
                flags.positional.push(arg.clone());
            }
            index += 1;
        }
        Ok(flags)
    }

    fn value(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    fn has(&self, key: &str) -> bool {
        self.switches.iter().any(|name| name == key)
    }
}

const VALUE_OPTIONS: &[&str] = &[
    "out", "job", "sheet", "layout", "cols", "rows", "margin", "gutter", "bleed", "creep", "marks",
    "fit", "copies", "in", "every", "report", "pages", "password",
];
const SWITCHES: &[&str] = &["duplex", "mirror-back", "once", "help", "version"];

fn run_command(args: &[String]) -> Result<(), Error> {
    let flags = Flags::parse(args, VALUE_OPTIONS, SWITCHES)?;
    if flags.has("help") {
        print!("{HELP}");
        return Ok(());
    }
    if flags.positional.len() != 1 {
        return Err(Error::Config(
            "run takes exactly one input file; put options before or after it".into(),
        ));
    }
    let input = PathBuf::from(&flags.positional[0]);
    let job = job_for(&flags)?;
    let output = flags
        .value("out")
        .map(PathBuf::from)
        .unwrap_or_else(|| sheeter::default_output(&input));
    let report = flags.value("report").map(PathBuf::from);
    let summary = sheeter::impose_reporting(&input, &output, &job, report.as_deref())?;
    println!(
        "{} -> {} on {} -> {}",
        count(summary.pages_in, "page", "pages"),
        count(summary.sides, "side", "sides"),
        count(summary.sheets, "sheet", "sheets"),
        summary.written.display()
    );
    if summary.overflowing > 0 {
        eprintln!(
            "warning: {} placed pages are larger than their tile and will be trimmed at the edge; check the scale",
            summary.overflowing
        );
    }
    Ok(())
}

/// A job file, with command line flags laid over the top.
fn job_for(flags: &Flags) -> Result<Job, Error> {
    let mut job = match flags.value("job") {
        Some(path) => Job::from_file(Path::new(path))?,
        None => Job::default(),
    };
    if let Some(value) = flags.value("sheet") {
        job.sheet = value.to_string();
    }
    if let Some(value) = flags.value("layout") {
        job.layout = value.to_string();
    }
    if let Some(value) = flags.value("cols") {
        job.cols = parse_u32(value, "cols")?;
    }
    if let Some(value) = flags.value("rows") {
        job.rows = parse_u32(value, "rows")?;
    }
    if let Some(value) = flags.value("margin") {
        job.margin = value.to_string();
    }
    if let Some(value) = flags.value("gutter") {
        job.gutter = value.to_string();
    }
    if let Some(value) = flags.value("bleed") {
        job.bleed = value.to_string();
    }
    if let Some(value) = flags.value("creep") {
        job.creep = value.to_string();
    }
    if let Some(value) = flags.value("fit") {
        job.fit = value.to_string();
    }
    if let Some(value) = flags.value("copies") {
        job.copies = parse_u32(value, "copies")?;
    }
    if let Some(value) = flags.value("pages") {
        job.pages = value.to_string();
    }
    if let Some(value) = flags.value("password") {
        job.password = value.to_string();
    }
    if let Some(value) = flags.value("marks") {
        job.marks = value
            .split(',')
            .map(|part| part.trim().to_string())
            .filter(|part| !part.is_empty())
            .collect();
    }
    if flags.has("duplex") {
        job.duplex = true;
    }
    if flags.has("mirror-back") {
        job.mirror_back = true;
    }
    Ok(job)
}

/// `1 page`, `2 pages`.
fn count(value: usize, one: &str, many: &str) -> String {
    if value == 1 {
        format!("{value} {one}")
    } else {
        format!("{value} {many}")
    }
}

fn parse_u32(raw: &str, name: &str) -> Result<u32, Error> {
    raw.trim()
        .parse()
        .map_err(|_| Error::Config(format!("{name} wants a whole number, not {raw:?}")))
}

fn watch_command(args: &[String]) -> Result<(), Error> {
    let flags = Flags::parse(args, VALUE_OPTIONS, SWITCHES)?;
    if flags.has("help") {
        print!("{HELP}");
        return Ok(());
    }
    let job_path = flags
        .value("job")
        .ok_or_else(|| Error::Config("watch needs --job <file>".into()))?;
    let inbox = flags
        .value("in")
        .ok_or_else(|| Error::Config("watch needs --in <dir>".into()))?;
    let done = flags
        .value("out")
        .ok_or_else(|| Error::Config("watch needs --out <dir>".into()))?;
    let every: u64 = flags
        .value("every")
        .map(|raw| parse_u32(raw, "every").map(u64::from))
        .transpose()?
        .unwrap_or(2);
    let once = flags.has("once");

    let inbox = PathBuf::from(inbox);
    let done = PathBuf::from(done);
    std::fs::create_dir_all(&done)?;
    if !inbox.is_dir() {
        return Err(Error::Config(format!(
            "{} is not a folder",
            inbox.display()
        )));
    }
    println!(
        "watching {} every {every}s, writing to {}",
        inbox.display(),
        done.display()
    );

    loop {
        let job = Job::from_file(Path::new(job_path))?;
        let mut waiting: Vec<PathBuf> = std::fs::read_dir(&inbox)?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"))
            })
            .filter(|path| {
                !path
                    .file_stem()
                    .is_some_and(|stem| stem.to_string_lossy().ends_with("-sheets"))
            })
            .collect();
        waiting.sort();

        for input in waiting {
            let stem = input
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "output".into());
            let output = done.join(format!("{stem}-sheets.pdf"));
            if is_up_to_date(&input, &output) {
                continue;
            }
            match sheeter::impose(&input, &output, &job) {
                Ok(summary) => {
                    println!(
                        "{} -> {} ({})",
                        input.file_name().unwrap_or_default().to_string_lossy(),
                        summary.written.display(),
                        count(summary.sides, "side", "sides")
                    );
                    if summary.overflowing > 0 {
                        eprintln!(
                            "warning: {} placed pages are larger than their tile and will be trimmed",
                            summary.overflowing
                        );
                    }
                }
                Err(err) => eprintln!("{}: {err}", input.display()),
            }
        }

        if once {
            return Ok(());
        }
        std::thread::sleep(Duration::from_secs(every));
    }
}

fn is_up_to_date(input: &Path, output: &Path) -> bool {
    let Ok(out_time) = std::fs::metadata(output).and_then(|meta| meta.modified()) else {
        return false;
    };
    match std::fs::metadata(input).and_then(|meta| meta.modified()) {
        Ok(in_time) => out_time >= in_time,
        Err(_) => false,
    }
}
