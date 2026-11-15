# push-wp

将本地 Markdown 发布到 WordPress 的桌面工具（Tauri + Rust，Windows）。

## 功能

- 编辑 / 预览 Markdown（GFM：表格、任务列表、代码块等）
- 发布 / 更新文章，正文本地图片自动上传（按文件名缓存，支持强制覆盖）
- 发布表单：标题、固定链接、发布时间、分类、标签、摘要、特色图片
- 分类 / 标签候选自动拉取站点已有词条，点击选用
- 文章管理：远程列表、搜索、详情、删除
- 多站点配置，主题跟随系统明暗

## 下载使用

从 Releases 下载便携包 `push-wp_x.x.x_x64-portable.zip`，解压后双击 `push-wp.exe`。

- 配置与缓存保存在程序同级的 `data\` 目录，**不读写任何系统目录**
- 删除整个文件夹即完全卸载
- 首次运行会把旧版本系统目录里的配置自动迁移过来

## 构建

需要 Rust（MSVC 工具链）与 Node.js：

```sh
npx tauri dev     # 开发调试
npx tauri build   # 打包 release
```

产物：`src-tauri/target/release/push-wp.exe`。配置与缓存始终保存在 exe 同级的 `data\` 目录，自包含、无系统目录依赖。

## 版本管理

同时修改 `Cargo.toml` 与 `tauri.conf.json` 的 `version`，提交后打同名 tag 并推送，再打包上传 Release 资产。