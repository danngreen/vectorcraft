use super::{Arc, Deserialize, Node, NodeId, Point, Serialize};

/// New art on a locked artboard ([`Document::new_art`]). Setting it changes the document in
/// place, so a selection change that sets it doesn't replace the document.
#[derive(Default)]
pub struct NewArtIds(pub(crate) std::sync::RwLock<Arc<std::collections::HashSet<NodeId>>>);

impl NewArtIds {
    /// The objects.
    pub fn get(&self) -> Arc<std::collections::HashSet<NodeId>> {
        self.0.read().map(|g| g.clone()).unwrap_or_default()
    }
    /// Make `ids` the objects.
    pub fn set(&self, ids: Arc<std::collections::HashSet<NodeId>>) {
        if let Ok(mut g) = self.0.write() {
            *g = ids;
        }
    }
}

impl Clone for NewArtIds {
    fn clone(&self) -> Self {
        NewArtIds(std::sync::RwLock::new(self.get()))
    }
}

/// Editing state, not document content: two documents are equal whatever new art they name.
impl PartialEq for NewArtIds {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl std::fmt::Debug for NewArtIds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("NewArtIds").field(&self.get()).finish()
    }
}

/// Opacity-mask editing mode: `object`'s mask art lives on the temporary `layer` while editing.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaskEdit {
    pub object: NodeId,
    pub layer: NodeId,
}

/// A saved view (View → New View…): zoom, centre and rotation, listed at the bottom of the View menu.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedView {
    pub name: String,
    pub center: Point,
    pub zoom: f64,
    #[serde(default)]
    pub rotation: f64,
}

impl SavedView {
    /// Most views a document keeps (the View menu lists every one).
    pub const MAX: usize = 25;
    /// Longest name, in characters.
    pub const MAX_NAME: usize = 255;

    /// `name` trimmed and cut to [`Self::MAX_NAME`] characters (`None` when blank).
    pub fn clean_name(name: &str) -> Option<String> {
        let name: String = name.trim().chars().take(Self::MAX_NAME).collect();
        (!name.is_empty()).then_some(name)
    }

    /// The first "View N" none of `saved` is named.
    pub fn default_name(saved: &[SavedView]) -> String {
        (1..=saved.len() + 1).map(|i| format!("View {i}")).find(|n| !saved.iter().any(|x| &x.name == n)).unwrap_or_default()
    }

    /// Can the canvas show this view: a finite centre and a finite, positive zoom.
    pub fn is_usable(&self) -> bool {
        self.center.x.is_finite() && self.center.y.is_finite() && self.zoom.is_finite() && self.zoom > 0.0
    }
}

/// A saved selection (Select → Save Selection…): the objects that were selected, listed by name at
/// the bottom of the Select menu (the first [`SavedSelection::MENU`]) and in the Saved Selections
/// panel (every one). Its objects may sit on any layers: recalled from the panel, they are selected
/// hidden or locked too, so they move, transform and delete together.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedSelection {
    pub name: String,
    pub objects: Vec<NodeId>,
}

impl SavedSelection {
    /// Most saved selections a document keeps (the Saved Selections panel lists every one).
    pub const MAX: usize = 1000;
    /// Saved selections the Select menu lists (the first ones).
    pub const MENU: usize = 25;
    /// Longest name, in characters.
    pub const MAX_NAME: usize = 255;

    /// The name of a copy of saved selection `name`: its name without a trailing number, then the
    /// number after the highest that a saved selection of `saved` named so has, 2 at least
    /// ("Knob" → "Knob 2", "Jack 12" → "Jack 13", "Knob 2" → "Knob 4" beside "Knob 3").
    pub fn next_name(name: &str, saved: &[SavedSelection]) -> String {
        let split = |name: &str| -> (String, Option<u64>) {
            match name.trim().rsplit_once(' ') {
                Some((base, n)) if !base.trim().is_empty() && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => {
                    (base.trim_end().to_string(), n.parse().ok())
                }
                _ => (name.trim().to_string(), None),
            }
        };
        let (base, own) = split(name);
        let highest = saved.iter().map(|s| split(&s.name)).filter(|(b, _)| *b == base).filter_map(|(_, n)| n).chain(own).max().unwrap_or(1);
        let base: String = base.chars().take(Self::MAX_NAME.saturating_sub(21)).collect();
        // The next number, then (past u64 or taken) a free one among the first MAX + 2.
        let next = highest.checked_add(1).map(|n| format!("{base} {}", n.max(2)));
        next.filter(|c| !saved.iter().any(|s| s.name == *c))
            .or_else(|| (2..=Self::MAX + 2).map(|n| format!("{base} {n}")).find(|c| !saved.iter().any(|s| s.name == *c)))
            .unwrap_or_else(|| format!("{base} copy"))
    }

    /// `name` trimmed and cut to [`Self::MAX_NAME`] characters (`None` when blank).
    pub fn clean_name(name: &str) -> Option<String> {
        let name: String = name.trim().chars().take(Self::MAX_NAME).collect();
        (!name.is_empty()).then_some(name)
    }

    /// The first "Selection N" none of `saved` is named.
    pub fn default_name(saved: &[SavedSelection]) -> String {
        (1..=saved.len() + 1).map(|i| format!("Selection {i}")).find(|n| !saved.iter().any(|x| &x.name == n)).unwrap_or_default()
    }
}

/// A symbol definition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    pub art: Arc<Node>,
}

/// Encoded image bytes (PNG/JPEG/WebP…) shared by image objects.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageBlob {
    pub mime: String,
    #[serde(skip)]
    pub bytes: Arc<Vec<u8>>,
    /// A linked image's low-resolution preview (PNG, see [`links`]): what a save writes when only
    /// linked images show this blob, and what `bytes` hold while the linked file can't be read.
    #[serde(skip)]
    pub proxy: Option<Arc<Vec<u8>>>,
}

/// Pair the ids of `original` and of every object inside it with those of the same objects in
/// `copy`, a copy of it with new ids ([`Document::reid`]: the same tree), into `out`.
pub fn copied_ids(original: &Node, copy: &Node, out: &mut std::collections::HashMap<NodeId, NodeId>) {
    out.insert(original.id, copy.id);
    if let (Some(a), Some(b)) = (original.children(), copy.children()) {
        for (a, b) in a.iter().zip(b) {
            copied_ids(a, b, out);
        }
    }
}
