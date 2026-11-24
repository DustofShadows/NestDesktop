use crate::{layout, model::Item};

#[derive(Clone, Debug, PartialEq)]
pub enum Intent {
    Before(String),
    End,
    Group(String),
    Outside,
}

/// The icon's center groups apps. The cell edges and label area are insertion
/// zones, so changing order never requires a modifier or a precision gesture.
pub fn intent(items: &[Item], page: usize, l: &layout::PopupLayout, x: f32, y: f32) -> Intent {
    if x < layout::GRID_X || x >= l.width - 12. || y < l.grid_y || y >= l.height - 54. {
        return Intent::Outside;
    }
    let col = ((x - layout::GRID_X) / l.cell_w).floor().clamp(0., 3.) as usize;
    let row = ((y - l.grid_y) / l.cell_h).floor() as usize;
    let offset = row * l.cols + col;
    let index = page * layout::PAGE_ITEMS + offset;
    let Some(item) = items.get(index) else {
        return Intent::End;
    };
    let r = layout::item_rect(l, offset);
    let rx = x - r.left;
    let ry = y - r.top;
    if rx < l.cell_w * 0.25 {
        return Intent::Before(item.id.clone());
    }
    if rx > l.cell_w * 0.75 || ry > 50. {
        return items
            .get(index + 1)
            .map(|it| Intent::Before(it.id.clone()))
            .unwrap_or(Intent::End);
    }
    Intent::Group(item.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn centers_group_and_edges_insert_without_modifiers() {
        let items = vec![Item::shortcut("A", "a.exe"), Item::shortcut("B", "b.exe")];
        let l = layout::popup_layout(2);
        assert_eq!(
            intent(&items, 0, &l, 20., 130.),
            Intent::Before(items[0].id.clone())
        );
        assert_eq!(
            intent(&items, 0, &l, 70., 130.),
            Intent::Group(items[0].id.clone())
        );
        assert_eq!(
            intent(&items, 0, &l, 120., 130.),
            Intent::Before(items[1].id.clone())
        );
        assert_eq!(intent(&items, 0, &l, 230., 130.), Intent::End);
        assert_eq!(intent(&items, 0, &l, 200., 60.), Intent::Outside);
    }
}
