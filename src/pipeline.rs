use crate::error::SpiderError;
use crate::item::Item;

/// Item 处理管线，对应 Scrapy 的 ITEM_PIPELINES。
///
/// 每个 Item 在被 parse 产出后，依次经过 Pipeline 处理。
/// Pipeline 可以：
/// - 修改 Item（清洗、补全字段）
/// - 丢弃 Item（返回 `Ok(false)`）
/// - 持久化（写数据库、文件等）
/// - 记录日志
///
/// 默认实现 `()` 表示空管线，不做任何处理。
/// 多个管线可通过元组组合：`(LogPipeline, StorePipeline)`
#[allow(async_fn_in_trait)]
pub trait Pipeline: Send + Sync {
    /// Spider 启动时调用一次
    async fn open(&self, _spider_name: &str) -> Result<(), SpiderError> {
        Ok(())
    }

    /// 处理单个 Item。返回 true 保留，false 丢弃。
    async fn process(&self, _item: &mut Item, _spider_name: &str) -> Result<bool, SpiderError> {
        Ok(true)
    }

    /// Spider 结束时调用一次
    async fn close(&self, _spider_name: &str) -> Result<(), SpiderError> {
        Ok(())
    }
}

/// 空管线，不做任何处理
impl Pipeline for () {}

/// 两个管线组合：依次执行 A 和 B
impl<A: Pipeline, B: Pipeline> Pipeline for (A, B) {
    async fn open(&self, spider_name: &str) -> Result<(), SpiderError> {
        self.0.open(spider_name).await?;
        self.1.open(spider_name).await
    }

    async fn process(&self, item: &mut Item, spider_name: &str) -> Result<bool, SpiderError> {
        if !self.0.process(item, spider_name).await? {
            return Ok(false);
        }
        self.1.process(item, spider_name).await
    }

    async fn close(&self, spider_name: &str) -> Result<(), SpiderError> {
        self.0.close(spider_name).await?;
        self.1.close(spider_name).await
    }
}
