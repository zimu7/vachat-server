# 使用文档

本目录使用 [mdBook](https://rust-lang.github.io/mdBook/) 生成静态 HTML 使用文档。

## 安装 mdBook

```bash
cargo install mdbook
```

## 本地预览

```bash
cd docs
mdbook serve --open
```

默认会启动本地预览服务，修改 `manuals/` 目录下的 Markdown 文件后会自动刷新。

## 构建静态页面

```bash
cd docs
mdbook build
```

构建结果会生成到：

```text
docs/book/
```

将 `docs/book/` 目录中的文件上传到服务器静态目录即可展示。

## 文档目录

左侧目录由 `manuals/SUMMARY.md` 维护。新增章节时，先在 `manuals/` 目录下创建 Markdown 文件，再把对应条目添加到 `SUMMARY.md`。
