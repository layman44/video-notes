import type { Job, ProcessingStep } from "./types";

export const initialJobs: Job[] = [
  {
    id: "rag-overview",
    title: "从零理解 RAG 的工作原理",
    platform: "bilibili",
    duration: "28:47",
    updatedAt: "今天 10:24",
    status: "completed",
    progress: 100,
    sourceUrl: "https://www.bilibili.com/video/BV1RAGDEMO",
  },
  {
    id: "rust-async",
    title: "Rust 异步编程完整指南",
    platform: "douyin",
    duration: "56:18",
    updatedAt: "今天 09:58",
    status: "processing",
    progress: 68,
    sourceUrl: "https://v.douyin.com/rust-demo/",
  },
  {
    id: "user-interview",
    title: "产品经理如何做好用户访谈",
    platform: "bilibili",
    duration: "34:12",
    updatedAt: "昨天 21:16",
    status: "paused",
    progress: 41,
    sourceUrl: "https://www.bilibili.com/video/BV1USERDEMO",
  },
];

export const completedSteps: ProcessingStep[] = [
  { id: "download", label: "音频下载", detail: "00:08", state: "completed" },
  { id: "transcribe", label: "语音转写", detail: "02:11", state: "completed" },
  { id: "summarize", label: "内容整理", detail: "03:15", state: "completed" },
];

export const noteMarkdown = `## 摘要

本视频从零开始讲解 RAG（Retrieval-Augmented Generation，检索增强生成）的工作原理。

内容涵盖为什么需要 RAG、检索与生成如何配合、常见实现误区，以及一个最小可用的实现流程。

通过具体示例帮助我们建立对 RAG 的系统性理解。

---

## 核心要点

- RAG 用于缓解大模型的知识局限与幻觉问题，将外部知识检索引入生成流程。
- 检索阶段负责找到与问题相关的高质量上下文，生成阶段基于上下文生成答案。
- 检索质量、上下文选择和提示设计是影响效果的关键因素。
- 评估应关注答案的正确性、可追溯性与相关性，而不仅是流畅度。

---

## 章节笔记

- [00:00] 为什么需要 RAG
- [06:42] 检索与生成如何配合
- [18:15] 常见实现误区`;

export const transcriptText = `[00:00] 大家好，今天我们从零开始理解 RAG 的工作原理。

[00:18] 在直接使用大语言模型时，我们经常遇到知识过期和回答幻觉的问题。

[06:42] RAG 把整个回答过程拆分为检索和生成两个阶段。

[18:15] 常见误区是只关注向量数据库，而忽略了内容切分、召回质量和评估体系。`;
