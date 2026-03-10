use crate::item::Item;
use crate::error::SpiderError;

pub trait ItemOutput {
    fn write(&mut self, _item: Item) -> Result<(), SpiderError> {
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct Collector {
    items: Vec<Item>,
}

impl Collector {
    pub fn items(&self) -> &[Item] {
        &self.items
    }
}

impl ItemOutput for Collector {
    fn write(&mut self, item: Item) -> Result<(), SpiderError> {
        self.items.push(item);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Value;

    #[test]
    fn collector_stores_items_in_memory() {
        let mut output = Collector::default();
        let item = Item::new().with_field("title", Value::String("post".to_string()));

        output.write(item.clone()).unwrap();

        assert_eq!(output.items(), &[item]);
    }
}
