use std::collections::VecDeque;

pub const CAPACITY: usize = 20;

#[derive(Clone, Debug)]
pub struct HistoryItem {
    pub raw: String,
    pub unicode: String,
    pub latex: String,
    pub omml: String,
    pub domain: String,
}

#[derive(Clone, Debug, Default)]
pub struct History {
    items: VecDeque<HistoryItem>,
}

impl History {
    pub fn push(&mut self, item: HistoryItem) {
        if self.items.len() == CAPACITY {
            self.items.pop_front();
        }
        self.items.push_back(item);
    }

    pub fn last(&self) -> Option<&HistoryItem> {
        self.items.back()
    }

    pub fn iter(&self) -> impl Iterator<Item = &HistoryItem> {
        self.items.iter().rev()
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(n: usize) -> HistoryItem {
        HistoryItem {
            raw: n.to_string(),
            unicode: n.to_string(),
            latex: n.to_string(),
            omml: n.to_string(),
            domain: "chemistry".into(),
        }
    }

    #[test]
    fn caps_at_twenty() {
        let mut h = History::default();
        for i in 0..30 {
            h.push(item(i));
        }
        assert_eq!(h.len(), 20);
        assert_eq!(h.last().unwrap().raw, "29");
        assert_eq!(h.iter().last().unwrap().raw, "10");
    }
}
