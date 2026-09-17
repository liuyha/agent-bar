# 第三方声明

AgentBar 自有代码采用 [MIT License](../LICENSE)。以下为参考实现及随仓库分发的组件、图标声明；第三方资源保留其原有许可与权利归属。

## CodexBar

Codex 额度采集、日志解析与缓存策略参照 [CodexBar](https://github.com/steipete/CodexBar)，参考提交为 `2d9334237e7c48cf18799c7302ce3229069f3a81`。AgentBar 使用 Rust 实现这些行为，没有打包其 Swift 源码或 CLI。上游版权与许可声明保留如下。

```text
MIT License

Copyright (c) 2026 Peter Steinberger

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## shadcn/ui

`src/components/ui/` 中的 Button、Checkbox、NativeSelect、Progress 基于 [shadcn/ui](https://github.com/shadcn-ui/ui) 源码，按 AgentBar 的紧凑布局与主题进行了调整。原始 MIT 许可证及版权声明保存在 [licenses/shadcn-ui-MIT.txt](../licenses/shadcn-ui-MIT.txt)。

## 服务图标与品牌

Codex 与 Claude 图标来自 5SVG，具体文件、原始下载地址及许可说明见 [图标来源](../src/assets/providers/SOURCES.md)。项目的 MIT 许可证不替代这些资源的许可或商标权利。品牌与商标归各自所有者所有，AgentBar 与 OpenAI、Anthropic 无隶属或背书关系。

## 其他依赖

JavaScript 与 Rust 依赖分别记录在 [package.json](../package.json)、[Cargo.toml](../src-tauri/Cargo.toml) 及各自的锁文件中，遵循各依赖随附的许可证。
