//! The tags applied to the text of a view, so that styling it again only
//! changes those that changed: removing and applying all tags of a long
//! text takes long, and GTK lays out all of it again.

use std::collections::HashSet;
use std::ops::Range;

use gtk::prelude::*;

/// A tag on characters of the text.
pub(super) type Tagged = (gtk::TextTag, Range<i32>);

/// The tags applied to a buffer, and how its text changed since.
#[derive(Debug, Default)]
pub(super) struct AppliedTags {
    tags: Vec<Tagged>,
    edits: Vec<Edit>,
}

/// Characters of the text replaced by others.
#[derive(Debug, Clone, Copy)]
struct Edit {
    at: i32,
    removed: i32,
    inserted: i32,
}

impl AppliedTags {
    /// Notes that `removed` characters at `at` were replaced by `inserted`
    /// ones, before the buffer changes.
    pub(super) fn edited(&mut self, at: i32, removed: i32, inserted: i32) {
        self.edits.push(Edit {
            at,
            removed,
            inserted,
        });
    }

    /// Changes the tags of `buffer` to `tags`. Removes all tags where the
    /// text changed and where tags go, then applies those that are new and
    /// those that were removed with them.
    pub(super) fn replace(&mut self, buffer: &gtk::TextBuffer, tags: Vec<Tagged>) {
        let edits = std::mem::take(&mut self.edits);
        let mut stale: Vec<Range<i32>> = edits
            .iter()
            .enumerate()
            .map(|(index, edit)| {
                let inserted = edit.at..edit.at + edit.inserted;
                moved(&inserted, &edits[index + 1..]).unwrap_or_else(|touched| touched)
            })
            .collect();
        let mut kept = HashSet::new();
        for (tag, range) in std::mem::take(&mut self.tags) {
            match moved(&range, &edits) {
                Ok(range) => {
                    kept.insert((tag, range));
                }
                Err(touched) => stale.push(touched),
            }
        }
        let wanted: HashSet<&Tagged> = tags.iter().collect();
        stale.extend(
            kept.iter()
                .filter(|tagged| !wanted.contains(tagged))
                .map(|(_, range)| range.clone()),
        );
        let stale = merged(stale);
        for range in &stale {
            let start = buffer.iter_at_offset(range.start);
            buffer.remove_all_tags(&start, &buffer.iter_at_offset(range.end));
        }
        for tagged in &tags {
            let (tag, range) = tagged;
            if !kept.contains(tagged) || overlaps(&stale, range) {
                let start = buffer.iter_at_offset(range.start);
                buffer.apply_tag(tag, &start, &buffer.iter_at_offset(range.end));
            }
        }
        self.tags = tags;
    }
}

impl Edit {
    fn touches(&self, range: &Range<i32>) -> bool {
        range.start <= self.at + self.removed && self.at <= range.end
    }

    /// Where `offset` is after the edit, at its start if it was removed.
    fn shift(&self, offset: i32) -> i32 {
        if offset <= self.at {
            offset
        } else if offset >= self.at + self.removed {
            offset - self.removed + self.inserted
        } else {
            self.at
        }
    }
}

/// Where the characters of `range` are after `edits`, if no edit touched
/// them. Otherwise, the error has all characters they may be on: GTK gives
/// inserted text the tags around it.
fn moved(range: &Range<i32>, edits: &[Edit]) -> Result<Range<i32>, Range<i32>> {
    let mut range = range.clone();
    let mut touched = false;
    for edit in edits {
        if edit.touches(&range) {
            touched = true;
            let start = edit.shift(range.start).min(edit.at);
            let end = edit.shift(range.end).max(edit.at + edit.inserted);
            range = start..end;
        } else {
            range = edit.shift(range.start)..edit.shift(range.end);
        }
    }
    if touched { Err(range) } else { Ok(range) }
}

/// `ranges` sorted, those that overlap or touch merged, without the empty.
fn merged(mut ranges: Vec<Range<i32>>) -> Vec<Range<i32>> {
    ranges.sort_by_key(|range| range.start);
    let mut merged: Vec<Range<i32>> = Vec::new();
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    merged.retain(|range| !range.is_empty());
    merged
}

/// Whether any of the sorted, apart `ranges` overlaps `range`.
fn overlaps(ranges: &[Range<i32>], range: &Range<i32>) -> bool {
    let index = ranges.partition_point(|other| other.end <= range.start);
    ranges
        .get(index)
        .is_some_and(|other| other.start < range.end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insert(at: i32, inserted: i32) -> Edit {
        Edit {
            at,
            removed: 0,
            inserted,
        }
    }

    fn delete(range: Range<i32>) -> Edit {
        Edit {
            at: range.start,
            removed: range.len() as i32,
            inserted: 0,
        }
    }

    #[test]
    fn tags_apart_from_edits_move_with_the_text() {
        assert_eq!(moved(&(0..5), &[insert(8, 3)]), Ok(0..5));
        assert_eq!(moved(&(10..15), &[insert(8, 3)]), Ok(13..18));
        assert_eq!(moved(&(10..15), &[delete(2..6)]), Ok(6..11));
        assert_eq!(moved(&(10..15), &[insert(8, 3), delete(0..2)]), Ok(11..16));
    }

    #[test]
    fn tags_at_edits_cover_what_they_may_be_on() {
        // Text inserted at either end may take the tag.
        assert_eq!(moved(&(0..5), &[insert(5, 3)]), Err(0..8));
        assert_eq!(moved(&(5..10), &[insert(5, 3)]), Err(5..13));
        assert_eq!(moved(&(0..10), &[insert(4, 3)]), Err(0..13));
        assert_eq!(moved(&(4..10), &[delete(2..6)]), Err(2..6));
        assert_eq!(moved(&(4..6), &[delete(2..8)]), Err(2..2));
        // Touched once, a tag stays touched as it moves on.
        assert_eq!(moved(&(0..5), &[insert(5, 3), insert(0, 2)]), Err(0..10));
    }

    #[test]
    fn ranges_are_merged_and_found() {
        let ranges = merged(vec![8..9, 0..2, 2..4, 6..6, 3..5]);
        assert_eq!(ranges, vec![0..5, 8..9]);
        assert!(overlaps(&ranges, &(4..6)));
        assert!(!overlaps(&ranges, &(5..8)));
        assert!(overlaps(&ranges, &(7..20)));
        assert!(!overlaps(&ranges, &(9..9)));
    }
}
