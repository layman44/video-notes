# VideoNotes 设计系统

视觉基线来自：

- `home-concept.png`
- `task-detail-concept.png`

## 设计方向

- 真白色主画布，冷灰色侧栏，不使用暖白或奶油色。
- 产品界面而非营销页面，依赖信息层级和留白形成视觉重点。
- 列表、开放画布和窄轨道优先，不使用卡片网格。
- 靛蓝色只用于主动作、选中状态、链接和进度。
- 推理、隐私和本地状态使用克制的文字与状态点表达。

## 核心令牌

```text
background          #FFFFFF
sidebar             #F6F8FC
surface-muted       #F8FAFD
text                #17191D
text-muted          #68707C
border              #E2E6EC
accent              #1D5FF2
accent-soft         #EAF0FF
success             #1D9A4A
warning             #E98312
danger              #D84343
sidebar-width       252px
statusbar-height    56px
radius-sm           8px
radius-md           12px
```

## 字体

- 中文 UI：Segoe UI Variable、Microsoft YaHei UI、PingFang SC。
- 正文：16px / 1.75。
- 控件：13px～15px，显式设置，不依赖浏览器默认。
- 主标题：36px / 1.2，字重 680。

## 允许的首屏文案

- VideoNotes
- 首页、任务、模型、设置
- 把视频变成可检索的笔记
- 粘贴抖音或哔哩哔哩链接，转写与整理均在本机完成。
- 粘贴视频链接或分享文本
- 开始解析
- 从剪贴板粘贴
- 最近任务
- 本地模型已就绪
- 数据不会上传

