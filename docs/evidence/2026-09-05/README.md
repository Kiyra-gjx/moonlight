# 2026 年 9 月 5 日验收材料

本目录保存代码版本 e31c812 的隔离演示验收结果。原团队的 data/moonlight.json 未参与采集，也未被修改。

- state-before.json：新生成默认演示数据的初始接口快照。
- state-after.json：执行一次自动派工后的接口快照；报告统计图的主要输入。
- decision.json：派工后任务 4 的决策矩阵。
- demo-after.json：交互验收结束并恢复任务状态后的隔离数据文件。
- checks.txt / checks.json：实际格式、Clippy、37 项测试和 Release 构建日志。
- api-tests.json / ui-tests.json：异常输入、桌面拖拽与移动端状态修改验收。
- source-sha256.txt：采集时 Rust 源码与内嵌前端的 SHA-256。
- PNG：根据这些数据绘制的统计图，以及真实页面截图、源码节选和日志视图。

## 从存档重新生成图表和报告

Python 依赖为 Pillow 与 python-docx。charts.py 使用 REPORT_FONT 指定中文字体文件，macOS 默认使用 Arial Unicode。

```bash
python3 scripts/report/charts.py
python3 scripts/report/build_report.py
```

## 重新采集

先运行 scripts/verify_report.py，再使用 scripts/report/capture.mjs。浏览器脚本需要 Playwright；可通过 PLAYWRIGHT_PATH 指定包目录，通过 CHROME_EXECUTABLE 指定 Chromium/Chrome 可执行文件。脚本会创建临时演示数据，服务监听 18089 端口，退出时关闭自己的服务和浏览器。

跨日期重新采集会改变预计交付日、延期损失与 NPV。应另存日期目录并同步报告，不应把新日志当作本次历史快照。

代码图是源码节选视图，日志图是实际输出的排版视图，不作为某成员独立完成对应文件的证明。
