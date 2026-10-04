//! Columns of text as the terminal shows them.

/// `rows` as lines, their columns two spaces apart and each as wide as its
/// widest value, those in `right` aligned to the right. No line ends in
/// spaces, so the last column takes only the room it needs.
pub fn table<const N: usize>(rows: &[[String; N]], right: &[usize]) -> String {
    let widths: Vec<usize> = (0..N)
        .map(|column| {
            rows.iter()
                .map(|row| row[column].chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    rows.iter()
        .map(|row| {
            let cells: Vec<String> = (0..N)
                .map(|column| {
                    let (value, width) = (&row[column], widths[column]);
                    if right.contains(&column) {
                        format!("{value:>width$}")
                    } else {
                        format!("{value:<width$}")
                    }
                })
                .collect();
            format!("{}\n", cells.join("  ").trim_end())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligns_columns() {
        let rows = [
            [
                "Webshop".to_owned(),
                "45 min".to_owned(),
                "Checkout".to_owned(),
            ],
            ["Café".to_owned(), "10 h 15 min".to_owned(), String::new()],
        ];
        assert_eq!(
            table(&rows, &[1]),
            "Webshop       45 min  Checkout\n\
             Café     10 h 15 min\n"
        );
        assert_eq!(table::<2>(&[], &[]), "");
    }
}
