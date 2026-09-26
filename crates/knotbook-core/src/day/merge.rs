//! Merging a conflict copy of a day into the day.

use super::Day;
use crate::Contradiction;
use crate::conflict::take;

impl Day {
    /// Takes from `other`, a conflict copy of this day, what does not
    /// contradict this day: blocks by their ids, if they do not overlap
    /// others, and each field and text that is the same on both sides or
    /// empty on one. Returns what contradicts, which stays as it is here.
    pub(crate) fn merge(&mut self, other: &Day) -> Vec<Contradiction> {
        let mut contradictions = Vec::new();
        let mut field = |name: &str, agreed: bool| {
            if !agreed {
                contradictions.push(Contradiction::DayField(name.to_owned()));
            }
        };
        field("kind", take(&mut self.kind, &other.kind, &String::new()));
        field("location", take(&mut self.location, &other.location, &None));
        field("tags", take(&mut self.tags, &other.tags, &Vec::new()));
        field("energy", take(&mut self.energy, &other.energy, &None));
        let mut work = (self.work_start, self.work_end);
        field(
            "work",
            take(
                &mut work,
                &(other.work_start, other.work_end),
                &(None, None),
            ),
        );
        (self.work_start, self.work_end) = work;
        for (name, value) in &other.unknown_fields {
            match self.unknown_fields.get(name) {
                None => {
                    self.unknown_fields.insert(name.clone(), value.clone());
                }
                Some(ours) => field(name, ours == value),
            }
        }

        let mut added = Vec::new();
        for theirs in &other.blocks {
            if let Some(ours) = self.blocks.iter_mut().find(|b| b.id == theirs.id) {
                let same_place = (ours.start, ours.end, &ours.project)
                    == (theirs.start, theirs.end, &theirs.project);
                if !same_place || !take(&mut ours.title, &theirs.title, &String::new()) {
                    contradictions.push(Contradiction::Block(ours.id.clone()));
                }
                if !take(&mut ours.text, &theirs.text, &String::new()) {
                    contradictions.push(Contradiction::BlockText(ours.id.clone()));
                }
                continue;
            }
            let (start, end) = theirs.span();
            let overlapped = self.blocks.iter().find(|ours| {
                let (ours_start, ours_end) = ours.span();
                ours_start < end && start < ours_end
            });
            match overlapped {
                Some(ours) => {
                    contradictions.push(Contradiction::Overlap(ours.id.clone(), theirs.id.clone()))
                }
                None => added.push(theirs.clone()),
            }
        }
        self.blocks.extend(added);
        self.blocks.sort_by_key(|block| block.span());

        if !take(&mut self.note, &other.note, &String::new()) {
            contradictions.push(Contradiction::DayNote);
        }
        contradictions
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
        assert_eq!(ours.merge(&theirs), []);
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
            ours.merge(&theirs),
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
        assert_eq!(ours.merge(&theirs), []);
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
            ours.merge(&theirs),
            [
                Contradiction::Block(id("aa11")),
                Contradiction::BlockText(id("aa11")),
                Contradiction::Overlap(id("aa11"), id("cc33")),
            ]
        );
    }
}
