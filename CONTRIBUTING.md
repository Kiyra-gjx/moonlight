# 协作约定

1. 从主分支拉取短期功能分支，命名建议为 `feature/功能名`、`fix/问题名` 或 `docs/文档名`。
2. 一次提交只解决一类问题，提交信息说明“做了什么”和“为什么”。
3. 合并前必须通过：

   ```bash
   cargo fmt --check
   cargo clippy --all-targets -- -D warnings
   cargo test
   ```

4. Pull Request 由至少一位非作者成员评审，重点检查需求、输入校验、异常数据和测试。
5. 真实分工和决策同步填入 `docs/成员分工与协作记录.md`。
