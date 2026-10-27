//! Reading source pages and writing imposed ones.
//!
//! Every source page becomes a Form XObject placed by a matrix. Objects the page
//! can reach are copied into the output document once, so fonts, images and
//! patterns keep working and nothing points at a document that no longer exists.

use std::collections::{HashMap, HashSet};

use lopdf::{Dictionary, Document, Object, ObjectId, Stream};

use crate::layout::{Placement, Side, Tile};
use crate::marks::{self, MarkStyle};
use crate::sheet::Rect;
use crate::units::num;
use crate::{Error, Fit};

/// The box a source page shows, after inheritance and rotation.
#[derive(Debug, Clone, Copy)]
pub struct PageBox {
    /// Visible width before rotation.
    pub width: f64,
    /// Visible height before rotation.
    pub height: f64,
    /// Clockwise rotation in degrees, always 0, 90, 180 or 270.
    pub rotate: i64,
}

impl PageBox {
    /// Size as a viewer shows it.
    fn shown(self) -> (f64, f64) {
        if self.rotate == 90 || self.rotate == 270 {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        }
    }
}

/// A 2D affine matrix in PDF order: `[a b c d e f]`.
#[derive(Debug, Clone, Copy)]
struct Mat {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    e: f64,
    f: f64,
}

impl Mat {
    const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    /// `self` applied after `inner`.
    fn then(self, inner: Self) -> Self {
        Self {
            a: self.a * inner.a + self.c * inner.b,
            b: self.b * inner.a + self.d * inner.b,
            c: self.a * inner.c + self.c * inner.d,
            d: self.b * inner.c + self.d * inner.d,
            e: self.a * inner.e + self.c * inner.f + self.e,
            f: self.b * inner.e + self.d * inner.f + self.f,
        }
    }

    fn translate(x: f64, y: f64) -> Self {
        Self {
            e: x,
            f: y,
            ..Self::IDENTITY
        }
    }

    fn scale(x: f64, y: f64) -> Self {
        Self {
            a: x,
            d: y,
            ..Self::IDENTITY
        }
    }

    fn to_ops(self) -> String {
        format!(
            "{} {} {} {} {} {} cm",
            num(self.a),
            num(self.b),
            num(self.c),
            num(self.d),
            num(self.e),
            num(self.f)
        )
    }
}

/// Rotation of a page box into an upright one.
fn unrotate(rotate: i64, width: f64, height: f64) -> Mat {
    match rotate {
        90 => Mat {
            a: 0.0,
            b: 1.0,
            c: -1.0,
            d: 0.0,
            e: height,
            f: 0.0,
        },
        180 => Mat {
            a: -1.0,
            b: 0.0,
            c: 0.0,
            d: -1.0,
            e: width,
            f: height,
        },
        270 => Mat {
            a: 0.0,
            b: -1.0,
            c: 1.0,
            d: 0.0,
            e: 0.0,
            f: width,
        },
        _ => Mat::IDENTITY,
    }
}

/// Builds an imposed document from a source.
pub struct Imposer<'a> {
    src: &'a Document,
    dst: Document,
    copied: HashMap<ObjectId, ObjectId>,
    forms: HashMap<usize, ObjectId>,
    boxes: HashMap<usize, PageBox>,
    /// The source pages chosen, in output order.
    page_ids: Vec<ObjectId>,
    /// Their original 1 based page numbers, for the report.
    numbers: Vec<u32>,
    /// Placements whose page is bigger than its tile, so the edges will be cut.
    overflowing: usize,
}

impl<'a> Imposer<'a> {
    /// `selection` holds 0 based source page indexes, in the order to use them.
    pub fn new(src: &'a Document, selection: Vec<usize>) -> Self {
        let all: Vec<ObjectId> = src.get_pages().values().copied().collect();
        let page_ids = selection
            .iter()
            .filter_map(|index| all.get(*index).copied())
            .collect();
        let numbers = selection.iter().map(|index| (*index + 1) as u32).collect();
        Self {
            src,
            dst: Document::with_version("1.7"),
            copied: HashMap::new(),
            forms: HashMap::new(),
            boxes: HashMap::new(),
            page_ids,
            numbers,
            overflowing: 0,
        }
    }

    #[must_use]
    pub fn page_count(&self) -> usize {
        self.page_ids.len()
    }

    /// Original page numbers, matching the placement indexes.
    #[must_use]
    pub fn numbers(&self) -> &[u32] {
        &self.numbers
    }

    /// Copy an object graph into the output, remapping every reference. Memoised,
    /// so a font shared by fifty pages is written once.
    fn copy_ref(&mut self, id: ObjectId) -> ObjectId {
        if let Some(done) = self.copied.get(&id) {
            return *done;
        }
        let new_id = self.dst.new_object_id();
        self.copied.insert(id, new_id);
        let source = self.src.get_object(id).cloned().unwrap_or(Object::Null);
        let remapped = self.remap(source, 0);
        self.dst.set_object(new_id, remapped);
        new_id
    }

    fn remap(&mut self, object: Object, depth: usize) -> Object {
        if depth > 64 {
            return Object::Null;
        }
        match object {
            Object::Reference(id) => Object::Reference(self.copy_ref(id)),
            Object::Array(items) => Object::Array(
                items
                    .into_iter()
                    .map(|item| self.remap(item, depth + 1))
                    .collect(),
            ),
            Object::Dictionary(dict) => Object::Dictionary(self.remap_dict(dict, depth + 1)),
            Object::Stream(mut stream) => {
                let dict = std::mem::replace(&mut stream.dict, Dictionary::new());
                stream.dict = self.remap_dict(dict, depth + 1);
                Object::Stream(stream)
            }
            other => other,
        }
    }

    fn remap_dict(&mut self, dict: Dictionary, depth: usize) -> Dictionary {
        let mut out = Dictionary::new();
        for (key, value) in dict.iter() {
            out.set(key.clone(), self.remap(value.clone(), depth));
        }
        out
    }

    /// Walk the page and its ancestors for an inheritable attribute.
    fn inherited(&self, page_id: ObjectId, key: &[u8]) -> Option<Object> {
        let mut current = page_id;
        let mut seen = HashSet::new();
        loop {
            if !seen.insert(current) {
                return None;
            }
            let dict = self.src.get_dictionary(current).ok()?;
            if let Ok(value) = dict.get(key) {
                return Some(self.deref(value.clone(), 0));
            }
            match dict.get(b"Parent") {
                Ok(Object::Reference(parent)) => current = *parent,
                _ => return None,
            }
        }
    }

    fn deref(&self, object: Object, depth: usize) -> Object {
        match object {
            Object::Reference(id) if depth < 32 => self
                .src
                .get_object(id)
                .cloned()
                .map(|inner| self.deref(inner, depth + 1))
                .unwrap_or(Object::Null),
            other => other,
        }
    }

    fn page_box(&mut self, index: usize) -> Result<PageBox, Error> {
        if let Some(found) = self.boxes.get(&index) {
            return Ok(*found);
        }
        let page_id = *self
            .page_ids
            .get(index)
            .ok_or_else(|| Error::Pdf(format!("page {index} does not exist")))?;

        let media = self
            .inherited(page_id, b"MediaBox")
            .and_then(|value| rect_of(&value))
            .ok_or_else(|| Error::Pdf(format!("page {index} has no usable MediaBox")))?;
        let crop = self
            .inherited(page_id, b"CropBox")
            .and_then(|value| rect_of(&value))
            .map(|rect| intersect(rect, media))
            .unwrap_or(media);
        if crop[2] - crop[0] <= 0.0 || crop[3] - crop[1] <= 0.0 {
            return Err(Error::Pdf(format!("page {index} has an empty crop box")));
        }
        let rotate = self
            .inherited(page_id, b"Rotate")
            .and_then(|value| match value {
                Object::Integer(v) => Some(v),
                Object::Real(v) => Some(i64::from(v as i32)),
                _ => None,
            })
            .unwrap_or(0)
            .rem_euclid(360);
        let rotate = match rotate {
            90 | 180 | 270 => rotate,
            _ => 0,
        };
        let found = PageBox {
            width: crop[2] - crop[0],
            height: crop[3] - crop[1],
            rotate,
        };
        self.boxes.insert(index, found);
        Ok(found)
    }

    /// Turn one source page into a Form XObject, content normalised to the origin.
    fn form(&mut self, index: usize) -> Result<ObjectId, Error> {
        if let Some(done) = self.forms.get(&index) {
            return Ok(*done);
        }
        let page_id = *self
            .page_ids
            .get(index)
            .ok_or_else(|| Error::Pdf(format!("page {index} does not exist")))?;
        let page_box = self.page_box(index)?;

        let media = self
            .inherited(page_id, b"MediaBox")
            .and_then(|value| rect_of(&value))
            .ok_or_else(|| Error::Pdf(format!("page {index} has no usable MediaBox")))?;
        let crop = self
            .inherited(page_id, b"CropBox")
            .and_then(|value| rect_of(&value))
            .map(|rect| intersect(rect, media))
            .unwrap_or(media);

        let resources = match self.inherited(page_id, b"Resources") {
            Some(Object::Reference(id)) => Object::Reference(self.copy_ref(id)),
            Some(Object::Dictionary(dict)) => Object::Dictionary(self.remap_dict(dict, 0)),
            _ => Object::Dictionary(Dictionary::new()),
        };

        let content = self.src.get_page_content(page_id);

        let mut body = Vec::with_capacity(content.len() + 32);
        body.extend_from_slice(
            format!("1 0 0 1 {} {} cm\n", num(-crop[0]), num(-crop[1])).as_bytes(),
        );
        body.extend_from_slice(&content);

        let mut dict = Dictionary::new();
        dict.set("Type", Object::Name(b"XObject".to_vec()));
        dict.set("Subtype", Object::Name(b"Form".to_vec()));
        dict.set("FormType", Object::Integer(1));
        dict.set(
            "BBox",
            Object::Array(vec![
                Object::Real(0.0),
                Object::Real(0.0),
                Object::Real(page_box.width as f32),
                Object::Real(page_box.height as f32),
            ]),
        );
        dict.set("Resources", resources);
        let stream = Stream::new(dict, body);
        let id = self.dst.add_object(stream);
        self.forms.insert(index, id);
        Ok(id)
    }

    /// Place one side's pages on a sheet and add the page to the output.
    fn render_side(
        &mut self,
        side: &Side,
        sheet: Rect,
        style: &MarkStyle,
        area: Rect,
        bleed: f64,
        fit: Fit,
    ) -> Result<ObjectId, Error> {
        let mut content = String::from("q\n");
        let mut xobjects = Dictionary::new();

        for Placement { page, tile, dx, dy } in &side.placements {
            let Some(index) = page else { continue };
            let form_id = self.form(*index)?;
            let page_box = self.page_box(*index)?;
            let (matrix, overflow) = placement(*tile, *dx, *dy, page_box, bleed, fit);
            if overflow {
                self.overflowing += 1;
            }
            let name = format!("Fm{}", form_id.0);
            xobjects.set(name.clone().into_bytes(), Object::Reference(form_id));
            content.push_str("q\n");
            content.push_str(&matrix.to_ops());
            content.push('\n');
            content.push_str(&format!("/{name} Do\n"));
            content.push_str("Q\n");
        }
        content.push_str(&marks::content(side, area, style));
        content.push_str("Q\n");

        let mut resources = Dictionary::new();
        resources.set("XObject", Object::Dictionary(xobjects));
        let content_id = self
            .dst
            .add_object(Stream::new(Dictionary::new(), content.into_bytes()));

        let mut page = Dictionary::new();
        page.set("Type", Object::Name(b"Page".to_vec()));
        page.set(
            "MediaBox",
            Object::Array(vec![
                Object::Real(0.0),
                Object::Real(0.0),
                Object::Real(sheet.w as f32),
                Object::Real(sheet.h as f32),
            ]),
        );
        page.set("Resources", Object::Dictionary(resources));
        page.set("Contents", Object::Reference(content_id));
        Ok(self.dst.add_object(page))
    }

    /// Render every planned side and write the file.
    pub fn write(
        mut self,
        sides: &[Side],
        prepared: &crate::Prepared,
        path: &std::path::Path,
    ) -> Result<usize, Error> {
        let catalog_id = self.dst.add_object(Dictionary::new());
        let pages_id = self.dst.add_object(Dictionary::new());

        let mut kids = Vec::with_capacity(sides.len());
        for side in sides {
            let page_id = self.render_side(
                side,
                prepared.sheet,
                &prepared.style,
                prepared.area,
                prepared.bleed,
                prepared.fit,
            )?;
            if let Ok(Object::Dictionary(dict)) = self.dst.get_object_mut(page_id) {
                dict.set("Parent", Object::Reference(pages_id));
            }
            kids.push(Object::Reference(page_id));
        }

        let mut pages_dict = Dictionary::new();
        pages_dict.set("Type", Object::Name(b"Pages".to_vec()));
        pages_dict.set("Count", Object::Integer(kids.len() as i64));
        pages_dict.set("Kids", Object::Array(kids));
        self.dst.set_object(pages_id, pages_dict);

        let mut catalog = Dictionary::new();
        catalog.set("Type", Object::Name(b"Catalog".to_vec()));
        catalog.set("Pages", Object::Reference(pages_id));
        self.dst.set_object(catalog_id, catalog);
        self.dst.trailer.set("Root", Object::Reference(catalog_id));

        self.check()?;
        self.dst.compress();
        self.dst
            .save(path)
            .map_err(|err| Error::Pdf(format!("could not write {}: {err}", path.display())))?;
        Ok(self.overflowing)
    }

    /// Every reference in the output must land on an object. Cheap, and it turns
    /// a silently broken file into an error before the file exists.
    fn check(&self) -> Result<(), Error> {
        for (id, object) in &self.dst.objects {
            let mut refs = Vec::new();
            collect_refs(object, &mut refs, 0);
            for target in refs {
                if !self.dst.objects.contains_key(&target) {
                    return Err(Error::Pdf(format!(
                        "object {}:{} points at a missing object {}:{}",
                        id.0, id.1, target.0, target.1
                    )));
                }
            }
        }
        let root = self
            .dst
            .trailer
            .get(b"Root")
            .and_then(Object::as_reference)
            .map_err(|_| Error::Pdf("the output has no catalogue".into()))?;
        if !self.dst.objects.contains_key(&root) {
            return Err(Error::Pdf("the catalogue was not written".into()));
        }
        Ok(())
    }
}

fn collect_refs(object: &Object, out: &mut Vec<ObjectId>, depth: usize) {
    if depth > 64 {
        return;
    }
    match object {
        Object::Reference(id) => out.push(*id),
        Object::Array(items) => {
            for item in items {
                collect_refs(item, out, depth + 1);
            }
        }
        Object::Dictionary(dict) => {
            for (_, value) in dict.iter() {
                collect_refs(value, out, depth + 1);
            }
        }
        Object::Stream(stream) => {
            for (_, value) in stream.dict.iter() {
                collect_refs(value, out, depth + 1);
            }
        }
        _ => {}
    }
}

/// Where one page goes: size it for the tile (plus bleed), then nudge. The flag
/// says the page came out bigger than its tile, so its edges will be trimmed off.
fn placement(tile: Tile, dx: f64, dy: f64, page: PageBox, bleed: f64, fit: Fit) -> (Mat, bool) {
    let (shown_w, shown_h) = page.shown();
    let box_w = tile.w + 2.0 * bleed;
    let box_h = tile.h + 2.0 * bleed;
    let (sx, sy) = match fit {
        Fit::Contain => {
            let scale = (box_w / shown_w).min(box_h / shown_h);
            (scale, scale)
        }
        Fit::Fill => (box_w / shown_w, box_h / shown_h),
        Fit::Scale(scale) => (scale, scale),
    };
    let draw_w = shown_w * sx;
    let draw_h = shown_h * sy;
    let ox = tile.x - bleed + (box_w - draw_w) / 2.0 + dx;
    let oy = tile.y - bleed + (box_h - draw_h) / 2.0 + dy;
    let overflow = draw_w > box_w + 0.5 || draw_h > box_h + 0.5;
    let matrix = Mat::translate(ox, oy)
        .then(Mat::scale(sx, sy))
        .then(unrotate(page.rotate, page.width, page.height));
    (matrix, overflow)
}

fn rect_of(object: &Object) -> Option<[f64; 4]> {
    let Object::Array(items) = object else {
        return None;
    };
    if items.len() != 4 {
        return None;
    }
    let mut out = [0.0; 4];
    for (slot, item) in out.iter_mut().zip(items) {
        *slot = match item {
            Object::Integer(v) => *v as f64,
            Object::Real(v) => f64::from(*v),
            _ => return None,
        };
    }
    Some([
        out[0].min(out[2]),
        out[1].min(out[3]),
        out[0].max(out[2]),
        out[1].max(out[3]),
    ])
}

fn intersect(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].min(b[2]),
        a[3].min(b[3]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_maps_fit_the_upright_box() {
        let page = PageBox {
            width: 100.0,
            height: 200.0,
            rotate: 90,
        };
        assert_eq!(page.shown(), (200.0, 100.0));
        let m = unrotate(90, 100.0, 200.0);
        // The corner (0,0) becomes (h, 0).
        assert!((m.e - 200.0).abs() < 1e-9);
        assert!((m.a).abs() < 1e-9);
    }

    #[test]
    fn placement_centres_a_landscape_page_on_a_portrait_tile() {
        let tile = Tile {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 200.0,
        };
        let page = PageBox {
            width: 200.0,
            height: 100.0,
            rotate: 0,
        };
        let (m, overflow) = placement(tile, 0.0, 0.0, page, 0.0, Fit::Contain);
        assert!(!overflow);
        // Scale is 0.5, so the drawn page is 100x50 and sits 75 points up.
        assert!((m.a - 0.5).abs() < 1e-9);
        assert!((m.f - 75.0).abs() < 1e-9);
    }
}
