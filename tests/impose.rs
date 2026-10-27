//! End to end: build a source, impose it, then read the output back and ask the
//! file itself which source page landed in which tile.

use std::path::{Path, PathBuf};

use lopdf::{Dictionary, Document, Object, Stream, dictionary};
use sheeter::{Job, impose};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("sheeter-integration");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir.join(name)
}

/// A source document whose pages say `P<n>`, so the output can be traced back.
fn fixture(path: &Path, pages: usize) {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });
    let mut kids = Vec::new();
    for n in 1..=pages {
        let content =
            format!("q 0.9 0.9 0.9 rg 40 40 515 712 re f Q\nBT /F1 36 Tf 72 700 Td (P{n}) Tj ET\n");
        let content_id = doc.add_object(Stream::new(Dictionary::new(), content.into_bytes()));
        let mut resources = Dictionary::new();
        resources.set("Font", dictionary! { "F1" => Object::Reference(font_id) });
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => Object::Reference(pages_id),
            "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            "Resources" => resources,
            "Contents" => Object::Reference(content_id),
        });
        kids.push(Object::Reference(page_id));
    }
    doc.set_object(
        pages_id,
        dictionary! {
            "Type" => "Pages",
            "Count" => pages as i64,
            "Kids" => kids,
        },
    );
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => Object::Reference(pages_id),
    });
    doc.trailer.set("Root", Object::Reference(catalog_id));
    doc.save(path).expect("save fixture");
}

/// The source page numbers placed on one output page, in drawing order.
///
/// Read from the page's own content stream, not from the resource dictionary: a
/// page that places the same form twice has one resource and two draw calls.
fn pages_on(doc: &Document, page_id: lopdf::ObjectId) -> Vec<u32> {
    let page = doc.get_dictionary(page_id).expect("page dict");
    let resources = page.get(b"Resources").and_then(Object::as_dict).ok();
    let Some(resources) = resources else {
        return Vec::new();
    };
    let xobjects = match resources.get(b"XObject") {
        Ok(Object::Dictionary(dict)) => dict.clone(),
        Ok(Object::Reference(id)) => doc.get_dictionary(*id).cloned().unwrap_or_default(),
        _ => return Vec::new(),
    };
    let content = String::from_utf8_lossy(&doc.get_page_content(page_id)).to_string();
    let mut out = Vec::new();
    for name in drawn_names(&content) {
        let Ok(value) = xobjects.get(name.as_bytes()) else {
            continue;
        };
        let Ok(id) = value.as_reference() else {
            continue;
        };
        let Ok(Object::Stream(stream)) = doc.get_object(id) else {
            continue;
        };
        let Ok(bytes) = stream.decompressed_content() else {
            continue;
        };
        let text = String::from_utf8_lossy(&bytes).to_string();
        if let Some(start) = text.find("(P") {
            let rest = &text[start + 2..];
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            if let Ok(n) = digits.parse() {
                out.push(n);
            }
        }
    }
    out
}

/// Every `/Name Do` in a content stream, in order.
fn drawn_names(content: &str) -> Vec<String> {
    let bytes = content.as_bytes();
    let mut out = Vec::new();
    let mut index = 0;
    while let Some(found) = content[index..].find("/Fm") {
        let start = index + found + 1;
        let mut end = start;
        while end < bytes.len() && bytes[end].is_ascii_alphanumeric() {
            end += 1;
        }
        let name = &content[start..end];
        let tail = &content[end..];
        if tail.trim_start().starts_with("Do") {
            out.push(name.to_string());
        }
        index = end.max(start);
        if index >= content.len() {
            break;
        }
    }
    out
}

fn side_contents(path: &Path) -> Vec<Vec<u32>> {
    let doc = Document::load(path).expect("reload output");
    doc.get_pages()
        .values()
        .map(|id| pages_on(&doc, *id))
        .collect()
}

#[test]
fn booklet_orders_the_signature() {
    let input = scratch("eight.pdf");
    let output = scratch("eight-booklet.pdf");
    fixture(&input, 8);
    let job = Job {
        sheet: "a4l".into(),
        layout: "booklet".into(),
        cols: 2,
        rows: 1,
        duplex: true,
        marks: vec!["crop".into()],
        margin: "8mm".into(),
        gutter: "6mm".into(),
        creep: "0.25mm".into(),
        ..Job::default()
    };
    let summary = impose(&input, &output, &job).expect("impose");
    assert_eq!(summary.pages_in, 8);
    assert_eq!(summary.sides, 4);
    assert_eq!(summary.sheets, 2);

    let sides = side_contents(&output);
    assert_eq!(sides.len(), 4);
    assert_eq!(sides[0], vec![8, 1], "outside front");
    assert_eq!(sides[1], vec![2, 7], "outside back");
    assert_eq!(sides[2], vec![6, 3], "inside front");
    assert_eq!(sides[3], vec![4, 5], "inside back");
}

#[test]
fn nup_places_every_page_once() {
    let input = scratch("sixteen.pdf");
    let output = scratch("sixteen-nup.pdf");
    fixture(&input, 16);
    let job = Job {
        sheet: "a3".into(),
        layout: "nup".into(),
        cols: 2,
        rows: 2,
        duplex: false,
        marks: vec!["none".into()],
        ..Job::default()
    };
    let summary = impose(&input, &output, &job).expect("impose");
    assert_eq!(summary.sides, 4);

    let mut seen: Vec<u32> = side_contents(&output).into_iter().flatten().collect();
    seen.sort_unstable();
    assert_eq!(seen, (1..=16).collect::<Vec<u32>>());
}

#[test]
fn duplex_nup_uses_a_fresh_block_on_the_back() {
    let input = scratch("sixteen-duplex.pdf");
    let output = scratch("sixteen-duplex-out.pdf");
    fixture(&input, 16);
    let job = Job {
        sheet: "a3".into(),
        layout: "nup".into(),
        cols: 2,
        rows: 2,
        duplex: true,
        mirror_back: true,
        marks: vec!["none".into()],
        ..Job::default()
    };
    let summary = impose(&input, &output, &job).expect("impose");
    assert_eq!(summary.sides, 4);
    assert_eq!(summary.sheets, 2);
    let mut seen: Vec<u32> = side_contents(&output).into_iter().flatten().collect();
    seen.sort_unstable();
    assert_eq!(seen, (1..=16).collect::<Vec<u32>>(), "every page exactly once");
}

#[test]
fn step_repeats_the_block_across_the_sheet() {
    let input = scratch("two.pdf");
    let output = scratch("two-step.pdf");
    fixture(&input, 2);
    let job = Job {
        sheet: "a4".into(),
        layout: "step".into(),
        cols: 2,
        rows: 2,
        marks: vec!["none".into()],
        ..Job::default()
    };
    impose(&input, &output, &job).expect("impose");
    let sides = side_contents(&output);
    assert_eq!(sides.len(), 1);
    assert_eq!(sides[0], vec![1, 2, 1, 2]);
}

#[test]
fn marks_without_a_margin_are_refused() {
    let input = scratch("refuse.pdf");
    fixture(&input, 4);
    let job = Job {
        margin: "0".into(),
        marks: vec!["crop".into()],
        ..Job::default()
    };
    let err = impose(&input, &scratch("never.pdf"), &job).expect_err("should refuse");
    let text = err.to_string();
    assert!(text.contains("margin"), "{text}");
}

#[test]
fn output_references_all_resolve() {
    let input = scratch("dangling.pdf");
    let output = scratch("dangling-out.pdf");
    fixture(&input, 12);
    let job = Job {
        sheet: "a3".into(),
        layout: "nup".into(),
        cols: 3,
        rows: 2,
        duplex: true,
        margin: "8mm".into(),
        gutter: "4mm".into(),
        marks: vec!["crop".into(), "reg".into()],
        ..Job::default()
    };
    impose(&input, &output, &job).expect("impose");

    let doc = Document::load(&output).expect("reload");
    let mut missing = Vec::new();
    for (id, object) in &doc.objects {
        let mut refs = Vec::new();
        walk(object, &mut refs, 0);
        for target in refs {
            if !doc.objects.contains_key(&target) {
                missing.push((*id, target));
            }
        }
    }
    assert!(missing.is_empty(), "dangling references: {missing:?}");
    // 12 pages at 6 up is two sides, and duplex puts them on one sheet.
    assert_eq!(doc.get_pages().len(), 2);
}

fn walk(object: &Object, out: &mut Vec<lopdf::ObjectId>, depth: usize) {
    if depth > 64 {
        return;
    }
    match object {
        Object::Reference(id) => out.push(*id),
        Object::Array(items) => items.iter().for_each(|item| walk(item, out, depth + 1)),
        Object::Dictionary(dict) => dict
            .iter()
            .for_each(|(_, value)| walk(value, out, depth + 1)),
        Object::Stream(stream) => stream
            .dict
            .iter()
            .for_each(|(_, value)| walk(value, out, depth + 1)),
        _ => {}
    }
}

#[test]
fn job_files_round_trip() {
    let job = Job::from_toml(
        r#"
        sheet = "320x450"
        layout = "step"
        cols = 3
        rows = 3
        margin = "6mm"
        gutter = "3mm"
        marks = ["crop", "reg"]
        "#,
    )
    .expect("valid job");
    assert_eq!(job.cols, 3);
    assert_eq!(job.marks.len(), 2);
    let prepared = job.prepare().expect("prepares");
    assert!(prepared.sheet.w > 900.0);
}

#[test]
fn unknown_keys_in_a_job_file_are_refused() {
    let err = Job::from_toml("sheat = \"A4\"").expect_err("typo must be caught");
    assert!(err.to_string().contains("sheat"), "{err}");
}
