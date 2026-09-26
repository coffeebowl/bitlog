//! Merging a conflict copy of a day into the day.

use super::{Block, Day};
use crate::Contradiction;
use crate::conflict::{Merger, take};

impl Day {
    /// Takes from `other`, a conflict copy of this day, what does not
    /// contradict this day: blocks by their ids, if they do not overlap
    /// others, and each field and text that is the same on both sides or
    /// empty on one. What contradicts is noted in `merger` and taken from
    /// `other` only if decided so.
    pub(crate) fn merge(&mut self, other: &Day, merger: &mut Merger) {
        let field = |name: &str| {
            let name = name.to_owned();
            move || Contradiction::DayField(name)
        };
        merger.value(&mut self.kind, &other.kind, &String::new(), field("kind"));
        merger.value(
            &mut self.location,
            &other.location,
            &None,
            field("location"),
        );
        merger.value(&mut self.tags, &other.tags, &Vec::new(), field("tags"));
        merger.value(&mut self.energy, &other.energy, &None, field("energy"));
        let mut work = (self.work_start, self.work_end);
        let theirs = (other.work_start, other.work_end);
        merger.value(&mut work, &theirs, &(None, None), field("work"));
        (self.work_start, self.work_end) = work;
        for (name, value) in &other.unknown_fields {
            let mut ours = self.unknown_fields.get(name).cloned();
            merger.value(&mut ours, &Some(value.clone()), &None, field(name));
            if let Some(ours) = ours {
                self.unknown_fields.insert(name.clone(), ours);
            }
        }

        let mut added = Vec::new();
        for theirs in &other.blocks {
            if let Some(ours) = self.blocks.iter_mut().find(|b| b.id == theirs.id) {
                let mut title = ours.title.clone();
                let same_place = (ours.start, ours.end, &ours.project)
                    == (theirs.start, theirs.end, &theirs.project);
                if same_place && take(&mut title, &theirs.title, &String::new()) {
                    ours.title = title;
                } else if merger.contradiction(Contradiction::Block(ours.id.clone())) {
                    ours.start = theirs.start;
                    ours.end = theirs.end;
                    ours.project = theirs.project.clone();
                    ours.title = theirs.title.clone();
                }
                let id = ours.id.clone();
                merger.value(&mut ours.text, &theirs.text, &String::new(), || {
                    Contradiction::BlockText(id)
                });
                continue;
            }
            let (start, end) = theirs.span();
            let overlaps = |ours: &Block| {
                let (ours_start, ours_end) = ours.span();
                ours_start < end && start < ours_end
            };
            if !self.blocks.iter().any(overlaps) {
                added.push(theirs.clone());
            } else if merger.contradiction(Contradiction::Overlap(theirs.id.clone())) {
                self.blocks.retain(|ours| !overlaps(ours));
                added.push(theirs.clone());
            }
        }
        self.blocks.extend(added);
        self.blocks.sort_by_key(|block| block.span());

        merger.value(&mut self.note, &other.note, &String::new(), || {
            Contradiction::DayNote
        });
    }
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, NaiveTime};

    use super::*;
    use crate::{Block, BlockId};

    fn time(value: &str) -> NaiveTime {
        value.parse().unwrap()
    }

    fn block(id: &str, start: &str, end: &str, title: &str, text: &str) -> Block {
        Block {
            id: id.parse().unwrap(),
            start: time(start),
            end: time(end),
            project: "infra".parse().unwrap(),
            title: title.to_owned(),
            text: text.to_owned(),
        }
    }

    fn day(blocks: Vec<Block>) -> Day {
        Day {
            blocks,
            ..Day::new(NaiveDate::from_ymd_opt(2026, 9, 22).unwrap())
        }
    }

    fn id(value: &str) -> BlockId {
        value.parse().unwrap()
    }

    fn merge(ours: &mut Day, theirs: &Day, chosen: &[Contradiction]) -> Vec<Contradiction> {
        let mut merger = Merger::new(chosen);
        ours.merge(theirs, &mut merger);
        merger.found
    }

    #[test]
    fn fields_empty_on_one_side_are_taken() {
        let mut ours = day(Vec::new());
        ours.energy = Some(3);
        let mut theirs = day(Vec::new());
        theirs.location = Some("office".parse().unwrap());
        theirs.work_start = Some(time("08:00"));
        theirs.work_end = Some(time("16:00"));
        theirs.note = "From the laptop".to_owned();
        theirs
            .unknown_fields
            .insert("mood".to_owned(), "calm".into());
        assert_eq!(merge(&mut ours, &theirs, &[]), []);
        assert_eq!(ours.energy, Some(3));
        assert_eq!(ours.location, theirs.location);
        assert_eq!(ours.work_end, Some(time("16:00")));
        assert_eq!(ours.note, "From the laptop");
        assert_eq!(ours.unknown_fields["mood"], "calm");
    }

    #[test]
    fn different_fields_contradict() {
        let mut ours = day(Vec::new());
        ours.energy = Some(3);
        ours.unknown_fields.insert("mood".to_owned(), "calm".into());
        ours.note = "Ours".to_owned();
        let mut theirs = ours.clone();
        theirs.kind = "vacation".to_owned();
        theirs.energy = Some(4);
        theirs
            .unknown_fields
            .insert("mood".to_owned(), "busy".into());
        theirs.note = "Theirs".to_owned();
        assert_eq!(
            merge(&mut ours, &theirs, &[]),
            [
                Contradiction::DayField("kind".to_owned()),
                Contradiction::DayField("energy".to_owned()),
                Contradiction::DayField("mood".to_owned()),
                Contradiction::DayNote,
            ]
        );
    }

    #[test]
    fn blocks_are_merged_by_id() {
        let mut ours = day(vec![
            block("aa11", "09:00", "10:00", "Deploy", ""),
            block("bb22", "10:00", "11:00", "", "Our text"),
        ]);
        let theirs = day(vec![
            block("aa11", "09:00", "10:00", "Deploy", "Their text"),
            block("bb22", "10:00", "11:00", "Review", "Our text"),
            block("cc33", "08:00", "09:00", "Early", ""),
        ]);
        assert_eq!(merge(&mut ours, &theirs, &[]), []);
        let ids: Vec<&str> = ours.blocks.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids, ["cc33", "aa11", "bb22"]);
        assert_eq!(ours.blocks[1].text, "Their text");
        assert_eq!(ours.blocks[2].title, "Review");
    }

    #[test]
    fn different_blocks_contradict() {
        let mut ours = day(vec![block("aa11", "09:00", "10:00", "Deploy", "Ours")]);
        let theirs = day(vec![
            block("aa11", "09:00", "10:30", "Deploy", "Theirs"),
            block("cc33", "09:45", "11:00", "Overlapping", ""),
        ]);
        assert_eq!(
            merge(&mut ours, &theirs, &[]),
            [
                Contradiction::Block(id("aa11")),
                Contradiction::BlockText(id("aa11")),
                Contradiction::Overlap(id("cc33")),
            ]
        );
    }

    #[test]
    fn contradictions_decided_for_the_copy() {
        let mut ours = day(vec![
            block("aa11", "09:00", "10:00", "Deploy", "Ours"),
            block("bb22", "10:00", "11:00", "Review", ""),
        ]);
        ours.energy = Some(3);
        let mut theirs = day(vec![
            block("aa11", "09:00", "09:30", "Deploy fix", "Theirs"),
            block("cc33", "10:30", "12:00", "Planning", ""),
        ]);
        theirs.energy = Some(4);
        let chosen = [
            Contradiction::DayField("energy".to_owned()),
            Contradiction::Block(id("aa11")),
            Contradiction::Overlap(id("cc33")),
        ];
        let found = merge(&mut ours, &theirs, &chosen);
        assert_eq!(
            found,
            [
                chosen[0].clone(),
                chosen[1].clone(),
                Contradiction::BlockText(id("aa11")),
                chosen[2].clone(),
            ]
        );
        assert_eq!(ours.energy, Some(4));
        // The text stays, it was not decided for the copy.
        assert_eq!(ours.blocks[0].title, "Deploy fix");
        assert_eq!(ours.blocks[0].end, time("09:30"));
        assert_eq!(ours.blocks[0].text, "Ours");
        // The overlapped block bb22 gives way.
        let ids: Vec<&str> = ours.blocks.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids, ["aa11", "cc33"]);
    }
}
