use crate::middleware::Chain;
use crate::store::SharedStore;
use crate::validator::StepValidator;

pub(crate) struct StepExecute {
    pub(crate) chain: Chain,
    pub(crate) stores: Vec<SharedStore>,
    pub(crate) step_validator: StepValidator,
}

impl StepExecute {
    pub(crate) fn new(
        chain: Chain,
        stores: Vec<SharedStore>,
        step_validator: StepValidator,
    ) -> Self {
        Self {
            chain,
            stores,
            step_validator,
        }
    }
}
