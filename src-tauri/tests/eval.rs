//! Chapter 5 evaluation runs for the BSc report (Testing Tracker sections 3 and 5).
//!
//! Every test here is `#[ignore]`d and driven by `KVC_EVAL_*` environment variables, so the
//! PowerShell scripts in the report's `Tracking/eval/` folder can call them. Run them by hand with:
//!
//! ```text
//! cargo test --release --test eval <name> -- --ignored --nocapture --exact
//! ```
//!
//! Each user-facing operation is timed as the desktop app or `kvc` runs it: a fresh `Repo::open`
//! plus the operation, inside the CPU budget pool (`cpu::install`). Run 0 is the warm-up and is
//! written to the CSV marked as run 0, but left out of the printed summary.
//!
//! Common variables:
//! - `KVC_EVAL_WORK`   folder for scratch stores (put it on the drive under test; default: temp)
//! - `KVC_EVAL_OUT`    CSV to append rows to (`config,label,item,run,value,unit`)
//! - `KVC_EVAL_CONFIG` config name written to each row (`L-SSD`, `L-HDD`, `D-NVMe`)
//! - `KVC_EVAL_LABEL`  data label written to each row (`P1`, `P2-stroke`, `S200`, ...)
//! - `KVC_EVAL_RUNS`   timed runs after the warm-up (default 5)
//! - `KVC_EVAL_BUDGET` CPU budget percent (default 75, the shipping default)

use krita_vc_lib::{branch, check, commands, commit, cpu, gc, repo};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

// --- plumbing ------------------------------------------------------------------------------

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.trim().is_empty())
}

fn env_or(name: &str, default: &str) -> String {
    env(name).unwrap_or_else(|| default.to_string())
}

fn env_num(name: &str, default: usize) -> usize {
    env(name)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(default)
}

fn work_dir(sub: &str) -> PathBuf {
    let base = env("KVC_EVAL_WORK")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("kvc-eval"));
    let dir = base.join(sub);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).expect("could not clear the previous scratch folder");
    }
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn apply_budget() -> u8 {
    let budget = env_num("KVC_EVAL_BUDGET", cpu::DEFAULT_BUDGET as usize).clamp(1, 100) as u8;
    cpu::set_budget(budget);
    budget
}

/// Rows in long format, appended to `KVC_EVAL_OUT`, plus an in-memory copy for the summary.
struct Log {
    config: String,
    label: String,
    rows: Vec<(String, usize, f64, String)>,
}

impl Log {
    fn new(default_label: &str) -> Log {
        Log {
            config: env_or("KVC_EVAL_CONFIG", "unknown"),
            label: env_or("KVC_EVAL_LABEL", default_label),
            rows: Vec::new(),
        }
    }

    fn add(&mut self, item: &str, run: usize, value: f64, unit: &str) {
        self.rows
            .push((item.to_string(), run, value, unit.to_string()));
    }

    fn ms(&mut self, item: &str, run: usize, t: Instant) {
        self.add(item, run, t.elapsed().as_secs_f64() * 1000.0, "ms");
    }

    /// Write the CSV and print median (min to max) of every timed item, warm-up excluded.
    fn finish(self) {
        if let Some(out) = env("KVC_EVAL_OUT") {
            let out = PathBuf::from(out);
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            let new = !out.exists();
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&out)
                .unwrap();
            if new {
                writeln!(f, "config,label,item,run,value,unit").unwrap();
            }
            for (item, run, value, unit) in &self.rows {
                writeln!(
                    f,
                    "{},{},{},{},{:.3},{}",
                    self.config, self.label, item, run, value, unit
                )
                .unwrap();
            }
            println!("rows appended to {}", out.display());
        }
        let mut timed: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
        for (item, run, value, unit) in &self.rows {
            if unit == "ms" && *run > 0 {
                timed.entry(item.as_str()).or_default().push(*value);
            }
        }
        println!(
            "\n[{} / {}] median (min to max), ms",
            self.config, self.label
        );
        for (item, mut v) in timed {
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let median = if v.len() % 2 == 1 {
                v[v.len() / 2]
            } else {
                (v[v.len() / 2 - 1] + v[v.len() / 2]) / 2.0
            };
            println!(
                "  {item:<28} {median:>10.1}  ({:.1} to {:.1}, n={})",
                v[0],
                v[v.len() - 1],
                v.len()
            );
        }
    }
}

/// Store size, split so the report can leave out the regenerable preview cache and the
/// clean-up quarantine (`trash/`), which are not history.
struct StoreSize {
    total: u64,
    cache: u64,
    trash: u64,
}

impl StoreSize {
    fn history(&self) -> u64 {
        self.total - self.cache - self.trash
    }
}

fn dir_bytes(path: &Path) -> u64 {
    let mut total = 0;
    if let Ok(rd) = std::fs::read_dir(path) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                total += dir_bytes(&p);
            } else if let Ok(m) = e.metadata() {
                total += m.len();
            }
        }
    }
    total
}

fn store_size(store: &Path) -> StoreSize {
    StoreSize {
        total: dir_bytes(store),
        cache: dir_bytes(&store.join("cache")),
        trash: dir_bytes(&store.join("trash")),
    }
}

fn log_store(log: &mut Log, prefix: &str, run: usize, store: &Path) {
    let s = store_size(store);
    log.add(
        &format!("{prefix}_store_total"),
        run,
        s.total as f64,
        "bytes",
    );
    log.add(
        &format!("{prefix}_store_history"),
        run,
        s.history() as f64,
        "bytes",
    );
    log.add(
        &format!("{prefix}_store_cache"),
        run,
        s.cache as f64,
        "bytes",
    );
    log.add(
        &format!("{prefix}_store_trash"),
        run,
        s.trash as f64,
        "bytes",
    );
}

/// Copy a revision into place and stamp it with the current time. `fs::copy` keeps the source's
/// modification time on Windows, so two same-sized revisions could otherwise look unchanged to a
/// size-and-time change check, as if the artist had not saved.
fn put(src: &Path, dst: &Path) {
    std::fs::copy(src, dst).unwrap();
    std::fs::File::options()
        .write(true)
        .open(dst)
        .unwrap()
        .set_modified(std::time::SystemTime::now())
        .unwrap();
}

/// Start tracking `doc` (if needed) and return its store folder.
fn track(doc: &Path) -> PathBuf {
    if !repo::Repo::is_repo(doc) {
        repo::Repo::init(doc).expect("could not start tracking the painting");
    }
    repo::Repo::open(doc).unwrap().store.clone()
}

/// Delete the preview cache so the next diff renders from the store, as on first opening.
fn clear_cache(store: &Path) {
    let cache = repo::cache_dir(store);
    if cache.exists() {
        let _ = std::fs::remove_dir_all(&cache);
    }
}

// --- timed operations: each is open + operation, as one user action -------------------------

fn op_commit(doc: &Path, message: &str) -> String {
    cpu::install(|| {
        let mut r = repo::Repo::open(doc).unwrap();
        commit::commit_snapshot(&mut r, message, "eval").unwrap().id
    })
}

/// The full visual diff of `commit_id` (layer list, composite and every layer's before/after
/// raster), against `against` or, by default, its first parent.
fn op_diff(doc: &Path, commit_id: &str, against: Option<&str>) {
    cpu::install(|| {
        let r = repo::Repo::open(doc).unwrap();
        let c = r.commits.iter().find(|c| c.id == commit_id).unwrap();
        let f = c
            .files
            .iter()
            .find(|f| f.is_kra)
            .expect("commit has no .kra");
        let base = against
            .map(str::to_string)
            .or_else(|| c.parents.first().cloned());
        let old = base
            .and_then(|b| commit::tree_at_commit(&r.commits, &b))
            .and_then(|t| t.get(&f.path).cloned());
        commands::committed_art_dto(&r, f, old.as_ref(), true, None).unwrap();
    })
}

fn op_restore(doc: &Path, commit_id: &str) {
    cpu::install(|| {
        let mut r = repo::Repo::open(doc).unwrap();
        commit::rollback_to_commit(&mut r, commit_id, "eval").unwrap();
    })
}

fn op_switch(doc: &Path, name: &str) {
    cpu::install(|| {
        let mut r = repo::Repo::open(doc).unwrap();
        branch::switch_branch(&mut r, name).unwrap();
    })
}

fn op_gc(doc: &Path) -> u64 {
    cpu::install(|| {
        let mut r = repo::Repo::open(doc).unwrap();
        gc::collect_garbage(&mut r, false).unwrap().bytes_reclaimed
    })
}

fn op_check(doc: &Path) -> bool {
    cpu::install(|| {
        let mut r = repo::Repo::open(doc).unwrap();
        let report = check::check_repository(&mut r, false).unwrap();
        for p in &report.problems {
            println!(
                "  check problem: {}",
                serde_json::json!({ "kind": p.kind, "detail": p.detail })
            );
        }
        report.ok()
    })
}

fn tip(doc: &Path) -> String {
    repo::Repo::open(doc)
        .unwrap()
        .branches
        .tip()
        .unwrap()
        .to_string()
}

fn require_file(var: &str) -> PathBuf {
    let p = PathBuf::from(env(var).unwrap_or_else(|| panic!("set {var}")));
    assert!(p.is_file(), "{var} is not a file: {}", p.display());
    p
}

// --- tests ---------------------------------------------------------------------------------

/// Start tracking one painting and commit it, so `kvc` (which cannot start tracking) has a
/// store to work on. `KVC_EVAL_DOC` = the `.kra`. Used by the CPU and FN-9 scripts.
#[test]
#[ignore = "evaluation: set KVC_EVAL_DOC"]
fn eval_prepare() {
    let doc = require_file("KVC_EVAL_DOC");
    let store = track(&doc);
    op_commit(&doc, "prepared for evaluation");
    println!("tracked {} (store {})", doc.display(), store.display());
}

/// CPU-5 and CPU-6: one commit of the pending change in `KVC_EVAL_DOC` (already tracked, edited
/// on disk) at `KVC_EVAL_BUDGET`. `kvc` itself always runs at the default budget, so this stands
/// in for it; like `kvc`, it first lowers its own process priority, so the only difference from
/// the docker's commit is the budget.
#[test]
#[ignore = "evaluation: set KVC_EVAL_DOC (tracked, with an edit pending)"]
fn eval_commit() {
    let doc = require_file("KVC_EVAL_DOC");
    cpu::lower_process_priority();
    let budget = apply_budget();
    let mut log = Log::new("commit");
    log.add("cpu_budget", 0, budget as f64, "percent");
    let t = Instant::now();
    op_commit(&doc, "budget run");
    log.add(
        "commit_once",
        1,
        t.elapsed().as_secs_f64() * 1000.0,
        "ms_once",
    );
    println!(
        "commit at {budget}% budget: {:.0} ms",
        t.elapsed().as_secs_f64() * 1000.0
    );
    log.finish();
}

/// PF-1 to PF-5, PF-7, PF-8 on one real edit. `KVC_EVAL_BEFORE` and `KVC_EVAL_AFTER` are two
/// saved revisions of the same painting. Every run starts from a fresh store:
/// first commit of BEFORE (PF-4), commit of the edit (PF-1/2/3), diff of the edit cold and warm
/// (PF-5), restore of the earlier version (PF-7), and a branch switch between them (PF-8).
/// `KVC_EVAL_KEEP_RESTORED` = a path to save the restored `.kra` from run 1, for FN-10.
#[test]
#[ignore = "evaluation: set KVC_EVAL_BEFORE and KVC_EVAL_AFTER"]
fn eval_ops() {
    let before = require_file("KVC_EVAL_BEFORE");
    let after = require_file("KVC_EVAL_AFTER");
    let runs = env_num("KVC_EVAL_RUNS", 5);
    let budget = apply_budget();
    let mut log = Log::new("pair");
    log.add("cpu_budget", 0, budget as f64, "percent");
    log.add(
        "before_file",
        0,
        std::fs::metadata(&before).unwrap().len() as f64,
        "bytes",
    );
    log.add(
        "after_file",
        0,
        std::fs::metadata(&after).unwrap().len() as f64,
        "bytes",
    );

    for run in 0..=runs {
        let dir = work_dir(&format!("ops-{run}"));
        let doc = dir.join("painting.kra");
        put(&before, &doc);
        let store = track(&doc);
        if run == 0 {
            println!("store: {}", store.display());
        }

        let t = Instant::now();
        let first = op_commit(&doc, "before");
        log.ms("first_commit", run, t);

        put(&after, &doc);
        let t = Instant::now();
        let edit = op_commit(&doc, "after");
        log.ms("commit_edit", run, t);

        clear_cache(&store);
        let t = Instant::now();
        op_diff(&doc, &edit, None);
        log.ms("diff_adjacent_cold", run, t);
        let t = Instant::now();
        op_diff(&doc, &edit, None);
        log.ms("diff_adjacent_warm", run, t);

        let t = Instant::now();
        op_restore(&doc, &first);
        log.ms("restore_earlier", run, t);
        if run == 1 {
            if let Some(keep) = env("KVC_EVAL_KEEP_RESTORED") {
                std::fs::copy(&doc, &keep).unwrap();
                println!("restored copy saved to {keep}");
            }
        }

        // main now holds BEFORE (the restore); a branch holding AFTER gives the switch real work.
        {
            let mut r = repo::Repo::open(&doc).unwrap();
            branch::create_branch(&mut r, "eval-side", None).unwrap();
        }
        put(&after, &doc);
        op_commit(&doc, "after, on a branch");
        let t = Instant::now();
        op_switch(&doc, "main");
        log.ms("switch_branch", run, t);

        log_store(&mut log, "final", run, &store);
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }
    log.finish();
}

/// RS-2, RS-6, RS-7 (krita-vc column) and PF-6. `KVC_EVAL_REVS` = a folder of numbered
/// revisions (`P1_r01.kra`, `P1_r02.kra`, ...), committed in name order into one store. Logs the
/// store size after every revision, then times the first-to-last diff and the health check, and
/// measures the store again after a clean-up.
#[test]
#[ignore = "evaluation: set KVC_EVAL_REVS"]
fn eval_replay() {
    let revs_dir = PathBuf::from(env("KVC_EVAL_REVS").expect("set KVC_EVAL_REVS"));
    let mut revs: Vec<PathBuf> = std::fs::read_dir(&revs_dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("kra")))
        .collect();
    revs.sort();
    assert!(
        revs.len() >= 2,
        "need at least two revisions in {}",
        revs_dir.display()
    );
    let runs = env_num("KVC_EVAL_RUNS", 5);
    apply_budget();
    let mut log = Log::new("replay");

    let dir = work_dir("replay");
    let doc = dir.join("painting.kra");
    put(&revs[0], &doc);
    let store = track(&doc);
    println!("store: {}", store.display());

    let mut ids = Vec::new();
    for (i, rev) in revs.iter().enumerate() {
        let n = i + 1;
        put(rev, &doc);
        log.add(
            "revision_file",
            n,
            std::fs::metadata(rev).unwrap().len() as f64,
            "bytes",
        );
        let t = Instant::now();
        ids.push(op_commit(&doc, &format!("r{n:02}")));
        // Logged as "bytes"-style series, not a timed item: one commit per revision.
        log.add(
            "replay_commit",
            n,
            t.elapsed().as_secs_f64() * 1000.0,
            "ms_once",
        );
        log_store(&mut log, "after_revision", n, &store);
        println!("  r{n:02} committed");
    }

    let (first, last) = (ids[0].clone(), ids[ids.len() - 1].clone());
    for run in 0..=runs {
        clear_cache(&store);
        let t = Instant::now();
        op_diff(&doc, &last, Some(&first));
        log.ms("diff_first_last_cold", run, t);
    }
    for run in 0..=runs {
        let t = Instant::now();
        assert!(op_check(&doc), "health check reported problems");
        log.ms("check", run, t);
    }

    clear_cache(&store);
    log_store(&mut log, "before_cleanup", 0, &store);
    let reclaimed = op_gc(&doc);
    log.add("cleanup_reclaimed", 0, reclaimed as f64, "bytes");
    log_store(&mut log, "after_cleanup", 0, &store);
    log.finish();
}

// --- S200: synthetic history -----------------------------------------------------------------

/// Deterministic LCG byte stream, incompressible like real LZF tile payloads (as in bench.rs).
struct Rng(u64);
impl Rng {
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n)
            .map(|_| {
                self.0 = self
                    .0
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (self.0 >> 33) as u8
            })
            .collect()
    }
}

type Layer = Vec<(i64, i64, Vec<u8>)>;

fn random_tile(rng: &mut Rng) -> Vec<u8> {
    let mut data = vec![0u8];
    data.extend(rng.bytes(64 * 64 * 4));
    data
}

fn synthetic_kra(layers: &[Layer], grid: i64) -> Vec<u8> {
    use zip::write::SimpleFileOptions;
    use zip::CompressionMethod;
    let px = grid * 64;
    let body: String = (0..layers.len())
        .map(|i| {
            format!(
                r#"<layer name="Layer {i}" uuid="l{i}" opacity="255" compositeop="normal" nodetype="paintlayer" filename="layer{i}"/>"#
            )
        })
        .collect();
    let maindoc = format!(
        r#"<!DOCTYPE DOC>
<DOC><IMAGE name="img" width="{px}" height="{px}"><layers>{body}</layers></IMAGE></DOC>"#
    );
    let mut out = Vec::new();
    {
        let mut zw = zip::ZipWriter::new(std::io::Cursor::new(&mut out));
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        zw.start_file("mimetype", opts).unwrap();
        zw.write_all(b"application/x-krita").unwrap();
        zw.start_file("maindoc.xml", opts).unwrap();
        zw.write_all(maindoc.as_bytes()).unwrap();
        for (i, tiles) in layers.iter().enumerate() {
            zw.start_file(format!("img/layers/layer{i}"), opts).unwrap();
            zw.write_all(
                format!(
                    "VERSION 2\nTILEWIDTH 64\nTILEHEIGHT 64\nPIXELSIZE 4\nDATA {}\n",
                    tiles.len()
                )
                .as_bytes(),
            )
            .unwrap();
            for (x, y, d) in tiles {
                zw.write_all(format!("{x},{y},LZF,{}\n", d.len()).as_bytes())
                    .unwrap();
                zw.write_all(d).unwrap();
            }
        }
        zw.finish().unwrap();
    }
    out
}

/// One "edit": replace about 5% of one layer's tiles.
fn edit(layers: &mut [Layer], round: usize, rng: &mut Rng) {
    let li = round % layers.len();
    let n = layers[li].len();
    for k in 0..(n / 20).max(1) {
        let idx = (round * 37 + k * 101) % n;
        let (x, y, _) = layers[li][idx];
        layers[li][idx] = (x, y, random_tile(rng));
    }
}

/// PF-9 to PF-13. Builds a synthetic history of `KVC_EVAL_VERSIONS` (default 200) versions on a
/// `KVC_EVAL_GRID` x `KVC_EVAL_GRID` tile canvas (default 40, so 2560 px) with three layers,
/// logging every build commit's time (cost against history length), then times at the end:
/// commit, diff, branch switch, clean-up (each run first makes a deleted branch to collect) and
/// the health check.
#[test]
#[ignore = "evaluation: synthetic history, long-running"]
fn eval_s200() {
    let versions = env_num("KVC_EVAL_VERSIONS", 200).max(2);
    let grid = env_num("KVC_EVAL_GRID", 40).max(2) as i64;
    let runs = env_num("KVC_EVAL_RUNS", 5);
    apply_budget();
    let mut log = Log::new("S200");
    log.add("versions", 0, versions as f64, "count");
    log.add("canvas_px", 0, (grid * 64) as f64, "px");

    let dir = work_dir("s200");
    let doc = dir.join("painting.kra");
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut layers: Vec<Layer> = (0..3)
        .map(|_| {
            let mut tiles = Vec::new();
            for ty in 0..grid {
                for tx in 0..grid {
                    tiles.push((tx * 64, ty * 64, random_tile(&mut rng)));
                }
            }
            tiles
        })
        .collect();
    std::fs::write(&doc, synthetic_kra(&layers, grid)).unwrap();
    log.add(
        "painting_file",
        0,
        std::fs::metadata(&doc).unwrap().len() as f64,
        "bytes",
    );
    let store = track(&doc);
    println!("store: {}", store.display());

    let build = Instant::now();
    for v in 1..=versions {
        if v > 1 {
            edit(&mut layers, v, &mut rng);
            std::fs::write(&doc, synthetic_kra(&layers, grid)).unwrap();
        }
        let t = Instant::now();
        op_commit(&doc, &format!("v{v}"));
        log.add(
            "build_commit",
            v,
            t.elapsed().as_secs_f64() * 1000.0,
            "ms_once",
        );
        if v % 20 == 0 {
            println!(
                "  {v} versions ({:.0} s so far)",
                build.elapsed().as_secs_f64()
            );
        }
    }
    log_store(&mut log, "after_build", versions, &store);

    let mut round = versions;
    for run in 0..=runs {
        round += 1;
        edit(&mut layers, round, &mut rng);
        std::fs::write(&doc, synthetic_kra(&layers, grid)).unwrap();
        let t = Instant::now();
        op_commit(&doc, &format!("v{round}"));
        log.ms("commit", run, t);
    }

    let latest = tip(&doc);
    for run in 0..=runs {
        clear_cache(&store);
        let t = Instant::now();
        op_diff(&doc, &latest, None);
        log.ms("diff_cold", run, t);
    }

    // A side branch one edit away from main; each run switches to the other branch.
    {
        let mut r = repo::Repo::open(&doc).unwrap();
        branch::create_branch(&mut r, "side", None).unwrap();
    }
    round += 1;
    edit(&mut layers, round, &mut rng);
    std::fs::write(&doc, synthetic_kra(&layers, grid)).unwrap();
    op_commit(&doc, "side edit");
    for run in 0..=runs {
        let target = if run % 2 == 0 { "main" } else { "side" };
        let t = Instant::now();
        op_switch(&doc, target);
        log.ms("switch_branch", run, t);
    }
    if repo::Repo::open(&doc).unwrap().branches.current != "main" {
        op_switch(&doc, "main");
    }

    for run in 0..=runs {
        // Something to collect: three versions on a branch that is then deleted.
        let name = format!("junk-{run}");
        {
            let mut r = repo::Repo::open(&doc).unwrap();
            branch::create_branch(&mut r, &name, None).unwrap();
        }
        for _ in 0..3 {
            round += 1;
            edit(&mut layers, round, &mut rng);
            std::fs::write(&doc, synthetic_kra(&layers, grid)).unwrap();
            op_commit(&doc, "junk");
        }
        op_switch(&doc, "main");
        {
            let mut r = repo::Repo::open(&doc).unwrap();
            branch::delete_branch(&mut r, &name).unwrap();
        }
        let t = Instant::now();
        let reclaimed = op_gc(&doc);
        log.ms("cleanup", run, t);
        log.add("cleanup_reclaimed", run, reclaimed as f64, "bytes");
    }

    for run in 0..=runs {
        let t = Instant::now();
        assert!(op_check(&doc), "health check reported problems");
        log.ms("check", run, t);
    }
    log_store(&mut log, "final", 0, &store);
    log.finish();
    let _ = std::fs::remove_dir_all(&dir);
}
