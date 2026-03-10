use crate::item::Item;

pub trait ItemOutput {
    fn write(&self, _item: &Item) -> Result<(), String> {
        Ok(())
    }
}
