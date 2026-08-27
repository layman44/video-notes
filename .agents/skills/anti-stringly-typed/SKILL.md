---
name: anti-stringly-typed
description: >-
  严格防范与消除 Stringly-Typed（字符串类型化反模式）与启发式文本匹配（Heuristic String Matching）。
  在编写、重构或审查代码时，指导使用强类型枚举、状态机与结构化错误码替代字符串包含判断。
---

# Anti-Stringly-Typed & 消除启发式匹配工程规范

## 1. 核心原则 (Core Principle)

> **“让编译器用类型（Enums/Types/Codes）来守护业务边界，永远不要让字符串去充当控制流的钥匙。”**

在代码修改、重构与审查时，必须对以下反模式进行 100% 拦截并重构：

---

## 2. 常见反模式与重构对照表 (Anti-Patterns vs. Refactored)

### ❌ 反模式 1：通过错误信息字符串包含判断错误类型
- **坏味道**：`if err.to_string().contains("下载") { ... }` 或 `if err.message.includes("404") { ... }`
- **问题**：自然语言文本一旦微调、文案多语言翻译或发生语义重叠，直接导致控制流误判。
- **✅ 正确工程化做法**：
  ```rust
  #[derive(Debug, PartialEq, Eq)]
  pub enum ModelError {
      NotInstalled,
      DownloadFailed(DownloadError),
      InferenceFailed(String),
  }
  ```
  通过 `match err` 或 `err.kind == ModelErrorKind::NotInstalled` 进行强类型分支判断。

---

### ❌ 反模式 2：通过名称子串猜测分类/能力
- **坏味道**：`if model_id.includes("embedding") { return "embedding"; }`
- **问题**：命名规则一旦变化或顺序颠倒（如 `qwen-embedding`），极易产生漏判或冲突。
- **✅ 正确工程化做法**：
  在数据模型中显式携带强类型分类字段：
  ```typescript
  export type ModelKind = "asr" | "summary" | "embedding" | "translation";
  export interface ModelStatus {
    id: string;
    kind: ModelKind; // 强类型定义，由后端权威发布
    installed: boolean;
  }
  ```

---

### ❌ 反模式 3：在前端对非结构化自然语言做二次反向解析
- **坏味道**：`if (viewStr.includes("万")) count = parseFloat(viewStr) * 10000;`
- **问题**：面对不同平台、多语言或边缘数据（如“千万”、“1.2M”）时解析脆弱。
- **✅ 正确工程化做法**：
  后端入库或传输时归一化为纯数值 `view_count: u64`，前端只负责展示格式化。

---

### ❌ 反模式 4：通过子进程 stdout/stderr 文本行包含做格式解析
- **坏味道**：`if line.contains("-->") { ... }`
- **问题**：内容本身包含 `-->`（如代码视频、数学逻辑）时引发格式误识别。
- **✅ 正确工程化做法**：
  使用严格的正则表达式（完整锚定）或状态机词法解析器。

---

## 3. 代码变更后的必检清单 (Code Review Checklist)

每次修改代码后，必须逐项自检：
- [ ] 是否存在针对 `error.message` / `error.to_string()` 的 `.contains()` / `.includes()` 判断？
- [ ] 是否存在针对业务 ID / 名称的前缀猜测？是否应由后端提供显式 `type` / `kind` 字段？
- [ ] 状态机流转是否使用了明确的枚举类型（Enum）而非随意赋值的 `String`？
- [ ] 是否有任何“面向人类日志”被错误用于“程序逻辑控制”？
