# Token 约等金额计价依据

核验日期：2026-09-16。所有价格单位均为 **USD / 1,000,000 tokens**。

统计金额按本机会话日志中的模型与 token 分类，折算为公开 **Standard API 价格等值**。它不代表 Codex / Claude 订阅账单、实际付款、剩余额度或节省金额。使用核验日价目表重新估算历史用量，不还原历史合同或促销账单。

## OpenAI

下列标准单价来自 [OpenAI 官方 API 定价](https://developers.openai.com/api/docs/pricing)。模型必须准确匹配，不可根据 `gpt-5` 等宽泛前缀推测价格。

| 模型 ID | 非缓存输入 | 缓存读取 | 缓存写入 | 输出 |
| --- | ---: | ---: | ---: | ---: |
| `gpt-6-astra` | 10 | 1 | 12.5 | 50 |
| `gpt-5.6-sol` | 4 | 0.4 | 5 | 20 |
| `gpt-5.6-terra` | 2 | 0.2 | 2.5 | 12 |
| `gpt-5.6-luna` | 0.2 | 0.02 | 0.25 | 1.2 |

这四个模型每次请求输入 **超过 272,000 tokens** 时，该请求的输入、缓存读取和缓存写入单价乘 2，输出单价乘 1.5。依据：[Astra](https://developers.openai.com/api/docs/models/gpt-6-astra)、[Sol](https://developers.openai.com/api/docs/models/gpt-5.6-sol)、[Terra](https://developers.openai.com/api/docs/models/gpt-5.6-terra)、[Luna](https://developers.openai.com/api/docs/models/gpt-5.6-luna)。

Codex 日志的 `input_tokens` 包含缓存读取和缓存写入，非缓存输入为 `input_tokens - cached_input_tokens - cache_write_input_tokens`。缓存不能重复计入输入成本，`reasoning_output_tokens` 也不能在 `output_tokens` 之外再次累加。

以下旧模型单价已分别核验；官方模型页没有单列缓存写入价，不能套用新模型的 1.25 倍规则。

| 模型 ID | 输入 | 缓存读取 | 输出 | 官方依据 |
| --- | ---: | ---: | ---: | --- |
| `gpt-5.5` | 5 | 0.5 | 30 | [模型页](https://developers.openai.com/api/docs/models/gpt-5.5) |
| `gpt-5.4` | 2.5 | 0.25 | 15 | [模型页](https://developers.openai.com/api/docs/models/gpt-5.4) |
| `gpt-5.3-codex` | 1.75 | 0.175 | 14 | [模型页](https://developers.openai.com/api/docs/models/gpt-5.3-codex) |

GPT-5.5 / GPT-5.4 官方页面对超过 272K 输入写明整个 session 的输入价格乘 2、输出乘 1.5；这与新模型页面的 full request 措辞不同。历史日志不足以可靠重建对应 API session 时，金额只能作为估算，不能声称恢复了完整结算规则。

## Claude

价格来自 [Anthropic 官方 API 定价](https://platform.claude.com/docs/en/about-claude/pricing)。精确 ID 和旧版本见 [模型版本规则](https://platform.claude.com/docs/en/about-claude/models/model-ids-and-versions)及[模型生命周期表](https://platform.claude.com/docs/en/about-claude/model-deprecations)。

| 精确模型 ID 或同价组 | 输入 | 缓存读取 | 5 分钟写入 | 1 小时写入 | 输出 |
| --- | ---: | ---: | ---: | ---: | ---: |
| `claude-fable-5-1` | 10 | 0.25 | 12.5 | 20 | 50 |
| `claude-fable-5` | 10 | 1 | 12.5 | 20 | 50 |
| `claude-opus-5`、`claude-opus-4-8`、`claude-opus-4-7`、`claude-opus-4-6` | 5 | 0.5 | 6.25 | 10 | 25 |
| `claude-opus-4-5`、`claude-opus-4-5-20251101` | 5 | 0.5 | 6.25 | 10 | 25 |
| `claude-opus-4-1-20250805`、`claude-opus-4-20250514` | 15 | 1.5 | 18.75 | 30 | 75 |
| `claude-sonnet-5` | 2 | 0.2 | 2.5 | 4 | 10 |
| `claude-sonnet-4-6`、`claude-sonnet-4-5`、`claude-sonnet-4-5-20250929`、`claude-sonnet-4-20250514` | 3 | 0.3 | 3.75 | 6 | 15 |
| `claude-haiku-4-5`、`claude-haiku-4-5-20251001` | 1 | 0.1 | 1.25 | 2 | 5 |
| `claude-3-5-haiku-20241022` | 0.8 | 0.08 | 1 | 1.6 | 4 |

Claude 4.6 及以后的模型在 1M 上下文范围内使用标准单价。当前 [Sonnet 4.5](https://platform.claude.com/docs/en/models/sonnet-4-5/overview) 与 [Opus 4.5](https://platform.claude.com/docs/en/models/opus-4-5/overview) 页面只列 200K 窗口；对更早 beta 的超长上下文日志不推测历史附加费。

Claude 的普通输入、缓存读取和缓存写入是独立分类。缓存写入按日志中的 5 分钟 / 1 小时分类计价；旧日志只有总写入量时，应明确采用默认 5 分钟缓存的估算口径。不得将总写入量再与 TTL 明细重复相加。

## 统计边界

- 没有可识别模型或已核实单价的记录（例如 `codex-auto-review`、自定义别名）保留 token、请求和轮次统计，但金额缺失；不能按免费或任意同族模型计价。存在无法计价的记录时，明确提示金额不完整或不可估算。
- 输入量、缓存分类、时间戳等日志缺失时，应保留不完整标记；读取失败不等于零用量。
- 日志只覆盖本机保留的会话，不能代表整个账号在其他电脑或云端的用量。日志中的请求响应 ID 用于去重；不能将 token 累计快照与单次请求明细同时相加。
- 金额采用标准 API 等值，不包含 Fast / Priority、Batch / Flex、地区附加费、工具单独计收的费用、税费、合同优惠、汇率换算或订阅折扣。这些费率与订阅实际消耗的映射不能从本机日志完整恢复。
- 日、周、月应按本机时区划分；本周从周一开始。请求次数统计可识别的模型调用；对话轮次统计用户发起的轮次，不等于内部工具调用次数。

更新模型或价格时，同时更新单价、精确模型映射、长上下文条件、核验日期与回归用例，保留未知模型的缺失金额行为。
