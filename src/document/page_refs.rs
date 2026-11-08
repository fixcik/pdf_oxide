//! Page index → page object reference, from one walk of the page tree.
//!
//! [`PdfDocument::get_page_ref`] used to walk the tree from the root on every
//! call, so asking for every page (annotations are read page by page) cost
//! O(pages²) object loads. The index below is built by a walk with exactly the
//! rules of [`PdfDocument::get_page_ref_recursive`]: the walk visits nodes in
//! the same order and its counting never depends on the target, so the i-th
//! `/Type /Page` node it meets is what `get_page_ref_recursive` returns for
//! target `i`. An index miss (no such page) falls back to the recursive walk,
//! which then fails exactly as before.

use super::*;

impl PdfDocument {
    /// The page references in page order, built once per document (see the
    /// module docs). `None` when the catalog or its `/Pages` entry is
    /// unusable; [`Self::get_page_ref`] then reports that error itself.
    pub(super) fn page_ref_index(&self) -> Option<Arc<Vec<ObjectRef>>> {
        // Built under the lock: concurrent first callers wait for one walk,
        // and `authenticate` cannot reset the slot between a walk and its
        // store. The walk only loads objects (`catalog`, `load_object`),
        // which never take this lock.
        let mut slot = self.page_ref_index.lock_or_recover();
        if let Some(index) = slot.as_ref() {
            return Some(Arc::clone(index));
        }
        let pages_ref = self.pages_root_ref().ok()?;
        let mut refs = Vec::new();
        self.collect_page_refs_like_lookup(pages_ref, &mut refs, &mut HashSet::new());
        Some(Arc::clone(slot.insert(Arc::new(refs))))
    }

    /// The catalog's `/Pages` reference, with `get_page_ref`'s errors.
    pub(super) fn pages_root_ref(&self) -> Result<ObjectRef> {
        let catalog = self.catalog()?;
        let catalog_dict = catalog.as_dict().ok_or_else(|| Error::InvalidObjectType {
            expected: "Dictionary".to_string(),
            found: catalog.type_name().to_string(),
        })?;
        catalog_dict
            .get("Pages")
            .ok_or_else(|| Error::InvalidPdf("Catalog missing /Pages entry".to_string()))?
            .as_reference()
            .ok_or_else(|| Error::InvalidPdf("/Pages is not a reference".to_string()))
    }

    /// Every page `get_page_ref_recursive` can reach, in its order: a node
    /// already visited, one that fails to load, is not a dictionary, has no
    /// `/Type` or an unknown one, or is a `/Pages` node without a `/Kids`
    /// array is skipped; kids that are not references are skipped.
    fn collect_page_refs_like_lookup(
        &self,
        node_ref: ObjectRef,
        out: &mut Vec<ObjectRef>,
        visited: &mut HashSet<ObjectRef>,
    ) {
        if !visited.insert(node_ref) {
            return;
        }
        let Ok(node) = self.load_object(node_ref) else {
            return;
        };
        let Some(node_dict) = node.as_dict() else {
            return;
        };
        match node_dict.get("Type").and_then(|t| t.as_name()) {
            Some("Page") => out.push(node_ref),
            Some("Pages") => {
                let Some(kids) = node_dict.get("Kids").and_then(|k| k.as_array()) else {
                    return;
                };
                for kid in kids {
                    if let Some(kid_ref) = kid.as_reference() {
                        self.collect_page_refs_like_lookup(kid_ref, out, visited);
                    }
                }
            },
            _ => {},
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PDF of the given object bodies (object `i + 1` = `objs[i]`, catalog
    /// first), with a correct xref table.
    fn build(objs: &[&str]) -> Vec<u8> {
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offs = Vec::new();
        for (i, body) in objs.iter().enumerate() {
            offs.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
        }
        let xref = pdf.len();
        pdf.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes(),
        );
        for o in offs {
            pdf.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objs.len() + 1
            )
            .as_bytes(),
        );
        pdf
    }

    /// The per-call walk, for comparison.
    fn walk(doc: &PdfDocument, i: usize) -> Result<ObjectRef> {
        let root = doc.pages_root_ref()?;
        doc.get_page_ref_recursive(root, i, &mut 0, &mut HashSet::new())
    }

    fn assert_same_as_walk(objs: &[&str], want_pages: usize) {
        let doc = PdfDocument::from_bytes(build(objs)).unwrap();
        let index = doc.page_ref_index().expect("index");
        assert_eq!(index.len(), want_pages, "pages reached: {:?}", index);
        for i in 0..want_pages + 3 {
            let old = walk(&doc, i).map_err(|e| e.to_string());
            let new = doc.get_page_ref(i).map_err(|e| e.to_string());
            assert_eq!(new, old, "page {i}");
            if i < want_pages {
                assert_eq!(new, Ok(index[i]));
            }
        }
    }

    const PAGE: &str = "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] >>";

    #[test]
    fn multi_level_tree() {
        assert_same_as_walk(
            &[
                "<< /Type /Catalog /Pages 2 0 R >>",
                "<< /Type /Pages /Kids [3 0 R 4 0 R 9 0 R] /Count 5 >>",
                "<< /Type /Pages /Kids [5 0 R 6 0 R] /Count 2 >>",
                "<< /Type /Pages /Kids [7 0 R 10 0 R] /Count 2 >>",
                PAGE,
                PAGE,
                PAGE,
                "<< /Type /Pages /Kids [] /Count 0 >>",
                PAGE,
                "<< /Type /Pages /Kids [8 0 R 11 0 R] /Count 1 >>",
                PAGE,
            ],
            5,
        );
    }

    #[test]
    fn broken_nodes_are_skipped_like_the_walk() {
        // 3: duplicate kid (counted once), 4: cycle back to the root, 5: no
        // /Type, 6: unknown /Type, 7: not a dictionary, 8: /Pages without
        // /Kids, 99: missing object, an inline (non-reference) kid.
        assert_same_as_walk(
            &[
                "<< /Type /Catalog /Pages 2 0 R >>",
                "<< /Type /Pages /Kids [3 0 R 3 0 R 4 0 R 5 0 R 6 0 R 7 0 R 8 0 R 99 0 R \
                 << /Type /Page >> 9 0 R 10 0 R] /Count 9 >>",
                PAGE,
                "<< /Type /Pages /Kids [2 0 R 9 0 R] /Count 1 >>",
                "<< /Parent 2 0 R /MediaBox [0 0 10 10] >>",
                "<< /Type /Template >>",
                "[1 2 3]",
                "<< /Type /Pages /Count 3 >>",
                PAGE,
                PAGE,
            ],
            3,
        );
    }

    #[test]
    fn unusable_root_keeps_the_walk_error() {
        let doc = PdfDocument::from_bytes(build(&[
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Page /MediaBox [0 0 10 10] >>",
        ]))
        .unwrap();
        // The root itself is a page: page 0 is the root, page 1 is missing.
        assert_eq!(doc.get_page_ref(0).unwrap(), ObjectRef::new(2, 0));
        assert_eq!(
            doc.get_page_ref(1).map_err(|e| e.to_string()),
            walk(&doc, 1).map_err(|e| e.to_string())
        );
    }
}
