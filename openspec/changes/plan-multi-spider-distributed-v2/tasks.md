# 任务清单

- [ ] 1.1 完成 `plan-multi-spider-distributed-v2` 的 proposal、design 与 spec 增量文档，确认这次 change 是 v2 规划，不在当前版本实现。
- [ ] 1.2 把多爬虫分布式的四层身份模型固定下来：`spider_name`、`job_id`、`worker_id`、`scope`。
- [ ] 1.3 明确 controller 与 Redis 的职责边界：controller 负责控制面，Redis 负责推荐协调数据面。

- [ ] 2.1 设计 spider registry 模型，明确一个 worker 如何按 `spider_name` 实例化 spider。
- [ ] 2.2 设计 job manifest / job registry 模型，明确 spider definition 与 job runtime 的分离。
- [ ] 2.3 设计 namespace 规则，明确常驻模式与批次模式下 scope 的命名策略。

- [ ] 3.1 在后续真正实现前，先补充多爬虫分布式的示例草图与部署文档结构。
- [ ] 3.2 在 `docs/distributed_scheduler.md` 与 `docs/operations.md` 中预留“当前已有分布式调度原语”和“v2 多爬虫控制面”之间的边界说明。
- [ ] 3.3 等 v1 request runtime 重构完成后，再拆具体实现任务：controller、worker、registry、job control API、cluster telemetry。
