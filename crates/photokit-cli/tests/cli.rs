//! End-to-end tests of the `photokit` binary: flags, output paths, exit codes.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_photokit"));
    c.env("PHOTOKIT_CONFIG_DIR", std::env::temp_dir().join("pk-cli-test-config"));
    c
}

fn workdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("pk-cli-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn photo(path: &Path, w: u32, h: u32) {
    image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x % 256) as u8, (y % 256) as u8, 90])).save(path).unwrap();
}

fn run(args: &[&str], dir: &Path) -> Output {
    bin().args(args).current_dir(dir).output().unwrap()
}

#[test]
fn resize_writes_next_to_source_without_touching_it() {
    let d = workdir("resize");
    photo(&d.join("a.jpg"), 400, 300);
    let out = run(&["resize", "a.jpg", "--width", "100", "--quiet"], &d);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(image::image_dimensions(d.join("a-resized.jpg")).unwrap(), (100, 75));
    assert_eq!(image::image_dimensions(d.join("a.jpg")).unwrap(), (400, 300));
    // A second run never overwrites: it numbers the new file.
    run(&["resize", "a.jpg", "--width", "100", "--quiet"], &d);
    assert!(d.join("a-resized-1.jpg").exists());
}

#[test]
fn convert_explicit_output_and_exists_exit_code() {
    let d = workdir("convert");
    photo(&d.join("a.png"), 64, 64);
    assert!(run(&["convert", "a.png", "-o", "b.webp", "--quiet"], &d).status.success());
    assert_eq!(image::ImageFormat::from_path(d.join("b.webp")).unwrap(), image::ImageFormat::WebP);
    let again = run(&["convert", "a.png", "-o", "b.webp"], &d);
    assert_eq!(again.status.code(), Some(4));
    assert!(run(&["convert", "a.png", "-o", "b.webp", "--overwrite", "--quiet"], &d).status.success());
}

#[test]
fn batch_json_report_and_partial_failure() {
    let d = workdir("batch");
    for i in 0..3 {
        photo(&d.join(format!("p{i}.jpg")), 300, 200);
    }
    std::fs::write(d.join("bad.jpg"), b"nope").unwrap();
    let out = run(&["batch", ".", "-W", "150", "-f", "png", "--name", "x-{n:2}", "-o", "out", "--json"], &d);
    assert_eq!(out.status.code(), Some(3), "some files fail -> exit 3");
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["succeeded"], 3);
    assert_eq!(report["failed"], 1);
    assert!(d.join("out/x-02.png").exists());
}

#[test]
fn rename_dry_run_then_apply() {
    let d = workdir("rename");
    for n in ["IMG_2.JPG", "IMG_1.JPG"] {
        photo(&d.join(n), 10, 10);
    }
    let dry = run(&["rename", ".", "-p", "trip-{n:2}", "--normalize-ext", "-n"], &d);
    assert!(dry.status.success());
    assert!(String::from_utf8_lossy(&dry.stdout).contains("IMG_1.JPG  ->  trip-01.jpg"));
    assert!(d.join("IMG_1.JPG").exists());
    assert!(run(&["rename", ".", "-p", "trip-{n:2}", "--normalize-ext"], &d).status.success());
    assert!(d.join("trip-01.jpg").exists() && d.join("trip-02.jpg").exists());
}

#[test]
fn usage_errors_exit_2_and_offline_backend_exits_5() {
    let d = workdir("codes");
    assert_eq!(run(&["resize"], &d).status.code(), Some(2));
    std::fs::create_dir_all(d.join("cfg")).unwrap();
    // Unreachable worker port.
    std::fs::write(d.join("cfg/config.toml"), "[worker]\nurl = \"http://127.0.0.1:9\"\n").unwrap();
    let out = bin()
        .args(["ai", "generate", "-p", "x", "--backend", "worker"])
        .env("PHOTOKIT_CONFIG_DIR", d.join("cfg"))
        .current_dir(&d)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(5), "{}", String::from_utf8_lossy(&out.stderr));
}
