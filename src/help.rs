//! The help page (docs/CLI.md §1). Commands first and lit; everything
//! else dimmer. Fits 60 cells so it reads the same in a narrow pane.

use crate::render::Renderer;
use crate::theme::Role;
use crate::util::fit;

pub fn page(r: &Renderer) {
    let t = &r.theme;
    let g = t.glyphs;
    let p = |s: &str| t.paint(Role::Plate, s);
    let d = |s: &str| t.paint(Role::Dupe, s);
    let k = |s: &str| t.paint_bold(Role::Keep, s);
    let i = |s: &str| t.paint(Role::Info, s);

    let row = |lead: String, text: &str| format!(" {}{}", lead, d(text));
    let c = |cmd: &str, text: &str| row(k(&fit(cmd, 8)), text);
    let tok = |t_: &str, text: &str| row(i(&fit(t_, 8)), text);
    let fl = |short: &str, long: &str, text: &str| {
        row(format!("{} {}", p(&fit(short, 3)), p(&fit(long, 16))), text)
    };

    let lines = vec![
        format!(
            " {}  {}",
            t.paint_bold(Role::Plate, "dedupe"),
            d("the deduplifier")
        ),
        t.rule(),
        format!(
            " {} {} {} {} {} {}",
            p("dedupe"),
            k("<command>"),
            i("[PATH]"),
            i("[.]"),
            i("[img|vid]"),
            d("[flags]")
        ),
        String::new(),
        c("find", "see every duplicate group, change nothing"),
        c(
            "milk",
            &format!(
                "move duplicates into PATH{}_milked",
                std::path::MAIN_SEPARATOR
            ),
        ),
        c(
            "purge",
            "verify bytes, then send duplicates to the Recycle Bin",
        ),
        c(
            "doctor",
            "check terminal, folder, journal, Recycle Bin, ffmpeg",
        ),
        String::new(),
        tok("PATH", "root folder                         default: here"),
        tok(
            ".",
            "include subfolders                  default: this folder only",
        ),
        tok("img", "images only; adds near-duplicate matching"),
        tok("vid", "videos only; adds near-duplicate matching (ffmpeg)"),
        String::new(),
        fl("-n", "--dry-run", "show what would change, change nothing"),
        fl(
            "-q",
            "--quick-search",
            "size + 64 KiB samples only, no full hash",
        ),
        fl("-u", "--undo", "reverse the last milk or purge under PATH"),
        fl("-dr", "--doctor", "same as the doctor command"),
        fl("-h", "--help", "this page"),
        String::new(),
        row(
            p(&fit("dedupe", 8)),
            "with nothing after it opens the workflow menu",
        ),
        row(
            p(&fit("keep", 8)),
            "oldest copy stays; shallower, then shorter path breaks ties",
        ),
        row(
            p(&fit("journal", 8)),
            "written only when milk or purge changes something",
        ),
        t.rule(),
        format!(
            " {} {}",
            d(g.bullet),
            d("find and dry runs never write; purge never hard-deletes")
        ),
    ];
    for l in lines {
        println!("{l}");
    }
}
