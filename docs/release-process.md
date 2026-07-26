# Release 流程

`.github/workflows/release.yml` 只响应 `v*` tag，不响应普通 dev/main push。
发布前必须同时满足：

1. tag 指向的提交是远端 `main` 的祖先；
2. tag 严格等于 `v` 加 `orbit-cli/Cargo.toml` 的三段版本，例如 `v0.1.0`；
3. 全工作区测试通过；
4. Windows MSI 与 Linux deb 都成功构建并通过各自校验。

推荐发布步骤：

```bash
# 先在合并到 main 的提交中更新版本与文档
git switch main
git pull --ff-only
git tag -a v0.1.0 -m "Orbit 0.1.0"
git push origin v0.1.0
```

workflow 会：

1. 在 `windows-latest` 通过仓库固定的 WiX 7 构建并验证 x64 MSI；
2. 在 `ubuntu-22.04` 原生构建 amd64 ELF，并用固定的 cargo-deb 3.7.0 生成 deb；
3. 汇总两个产物并生成 `SHA256SUMS`；
4. 使用 GitHub Release Notes API 与 `.github/release.yml` 的类别生成变更说明；
5. 在说明开头写明安装包名称、校验方式和 MSI 签名状态，然后发布 GitHub Release。

发布 job 只有 `contents: write` 权限；构建 job 仅有 `contents: read`。它使用
`--verify-tag`，不会替缺失 tag 创建新 tag。Release 创建成功后重新运行同一 workflow
会因 Release 已存在而失败，不会静默覆盖已发布资产。

当前 MSI 未签名。配置项目代码签名证书之前，Release note 会明确披露这一点。
