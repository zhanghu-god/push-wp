/* Push Markdown Rust —— 前端逻辑（零构建，直接使用 Tauri 全局 API） */

const { invoke, convertFileSrc } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { open, save, confirm: dialogConfirm } = window.__TAURI__.dialog;

const $ = (id) => document.getElementById(id);
const systemDark = window.matchMedia('(prefers-color-scheme: dark)');

const state = {
  filePath: null,
  modified: false,
  settings: { sites: [], theme: 'system' },
  publishing: false,
  detail: null,
  terms: { categories: [], tags: [] },
};

// ---------- 主题 ----------

function applyTheme() {
  const theme = state.settings.theme || 'system';
  const dark = theme === 'dark' || (theme === 'system' && systemDark.matches);
  document.documentElement.dataset.theme = dark ? 'dark' : 'light';
  document.querySelectorAll('[data-theme-choice]').forEach((btn) => {
    const selected = btn.dataset.themeChoice === theme;
    btn.classList.toggle('active', selected);
    btn.setAttribute('aria-pressed', String(selected));
  });
}

async function saveTheme(theme) {
  const previous = state.settings.theme;
  state.settings.theme = theme;
  applyTheme();
  try {
    await invoke('save_settings', { settings: state.settings });
  } catch (e) {
    state.settings.theme = previous;
    applyTheme();
    setStatus(`保存外观失败：${e}`);
  }
}

function toggleAppearance(force) {
  const panel = $('appearance-panel');
  const shouldOpen = typeof force === 'boolean' ? force : panel.hidden;
  panel.hidden = !shouldOpen;
}

// ---------- 状态栏 ----------

function setStatus(text) {
  $('status-text').textContent = text;
}

// ---------- 文件打开 / 保存 ----------

async function openFile() {
  const path = await open({
    multiple: false,
    filters: [{ name: 'Markdown', extensions: ['md', 'markdown', 'txt'] }],
  });
  if (!path) return;
  try {
    const res = await invoke('read_markdown', { path });
    state.filePath = path;
    state.modified = false;
    $('editor').value = res.content;
    $('welcome').hidden = true;
    $('split').hidden = false;
    $('btn-save').disabled = false;
    $('btn-publish').disabled = false;
    renderPreview(res.html);
    updateFileLabel();
    setStatus(`已打开：${path}`);
  } catch (e) {
    setStatus(`打开失败：${e}`);
    alert(`打开文件失败：\n${e}`);
  }
}

async function saveFile() {
  if (!state.filePath) {
    const path = await save({
      filters: [{ name: 'Markdown', extensions: ['md'] }],
    });
    if (!path) return;
    state.filePath = path;
  }
  try {
    await invoke('save_file', { path: state.filePath, content: $('editor').value });
    state.modified = false;
    updateFileLabel();
    setStatus(`已保存：${state.filePath}`);
  } catch (e) {
    setStatus(`保存失败：${e}`);
  }
}

function updateFileLabel() {
  const label = $('file-label');
  label.textContent = state.filePath || ($('split').hidden ? '未打开文件' : '未命名文档');
  label.classList.toggle('modified', state.modified);
}

// ---------- 预览 ----------

function renderPreview(html) {
  const tpl = document.createElement('template');
  tpl.innerHTML = html;
  // 本地图片走 Tauri asset 协议才能在 WebView 中显示
  tpl.content.querySelectorAll('img').forEach((img) => {
    const src = img.getAttribute('src') || '';
    if (src && !/^(https?:|data:)/.test(src)) {
      img.setAttribute('src', convertFileSrc(src));
    }
  });
  // 外链在新窗口打开（WebView 内跳转无意义）
  tpl.content.querySelectorAll('a[href^="http"]').forEach((a) => {
    a.setAttribute('target', '_blank');
    a.setAttribute('rel', 'noopener');
  });
  $('preview').innerHTML = tpl.innerHTML;
}

let renderTimer = null;
function scheduleRender() {
  state.modified = true;
  updateFileLabel();
  clearTimeout(renderTimer);
  renderTimer = setTimeout(async () => {
    try {
      const res = await invoke('render_markdown', {
        content: $('editor').value,
        path: state.filePath || 'untitled.md',
      });
      renderPreview(res.html);
    } catch (e) {
      setStatus(`渲染失败：${e}`);
    }
  }, 400);
}

// ---------- 设置 ----------

async function loadSettings() {
  try {
    state.settings = await invoke('get_settings');
  } catch (e) {
    setStatus(`读取设置失败：${e}`);
  }
}

function openSettings() {
  renderSiteList();
  $('settings-modal').hidden = false;
}

function renderSiteList() {
  const list = $('site-list');
  list.innerHTML = '';
  state.settings.sites.forEach((site, i) => {
    const card = document.createElement('div');
    card.className = 'site-card';
    card.innerHTML = `
      <div class="site-head">
        <strong>站点 ${i + 1}</strong>
        <button class="del-site" data-index="${i}">删除</button>
      </div>
      <div class="grid">
        <label>名称</label><input type="text" data-field="name" />
        <label>XML-RPC</label><input type="text" data-field="url" placeholder="https://example.com/xmlrpc.php" />
        <label>用户名</label><input type="text" data-field="username" />
        <label>密码</label><input type="password" data-field="password" />
      </div>`;
    card.querySelectorAll('input').forEach((input) => {
      input.value = site[input.dataset.field] || '';
      input.addEventListener('input', () => {
        state.settings.sites[i][input.dataset.field] = input.value;
      });
    });
    card.querySelector('.del-site').addEventListener('click', () => {
      state.settings.sites.splice(i, 1);
      renderSiteList();
    });
    list.appendChild(card);
  });
}

async function saveSettings() {
  try {
    await invoke('save_settings', { settings: state.settings });
    $('settings-modal').hidden = true;
    setStatus('设置已保存');
  } catch (e) {
    alert(`保存设置失败：\n${e}`);
  }
}

// ---------- 发布 ----------

function defaultTitleFromPath(p) {
  if (!p) return '新文章';
  const name = p.split(/[\\/]/).pop() || '';
  return name.replace(/\.(md|markdown|txt)$/i, '') || '新文章';
}

function nowLocalInput() {
  const d = new Date();
  const pad = (n) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

async function pickThumbnail() {
  const p = await open({
    multiple: false,
    filters: [{ name: '图片', extensions: ['png', 'jpg', 'jpeg', 'webp', 'gif', 'bmp', 'ico', 'tif', 'tiff'] }],
  });
  if (p) $('pub-thumb').value = p;
}

async function openPublish(presetId) {
  if ($('split').hidden) {
    alert('请先打开或新建一个文档');
    return;
  }
  if (state.settings.sites.length === 0) {
    alert('请先在设置中添加站点');
    openSettings();
    return;
  }
  // 表单默认值
  $('pub-title-input').value = defaultTitleFromPath(state.filePath);
  $('pub-slug-input').value = '';
  $('pub-date').value = nowLocalInput();
  $('pub-cats').value = '';
  $('pub-tags').value = '';
  $('pub-abstract').value = '';
  $('pub-thumb').value = '';
  const siteSel = $('pub-site');
  siteSel.innerHTML = '';
  state.settings.sites.forEach((site, i) => {
    const opt = document.createElement('option');
    opt.value = i;
    opt.textContent = `${site.name}（${site.username}）`;
    siteSel.appendChild(opt);
  });
  $('pub-mode').value = 'auto';
  $('row-input-id').hidden = true;
  $('pub-input-id').value = '';
  if (presetId) {
    $('pub-mode').value = 'manual';
    $('row-input-id').hidden = false;
    $('pub-input-id').value = presetId;
  }
  $('pub-forced').checked = false;
  $('pub-log').hidden = true;
  $('pub-log').innerHTML = '';
  $('btn-do-publish').disabled = false;
  $('publish-modal').hidden = false;
  await refreshCachedId();
  loadTerms(currentPubSite());
}

function currentPubSite() {
  return state.settings.sites[Number($('pub-site').value)];
}

/* 拉取站点已有分类/标签，渲染为可点击候选标签 */
async function loadTerms(site) {
  renderChips('pub-cats', 'pub-cats-chips', []);
  renderChips('pub-tags', 'pub-tags-chips', []);
  setTermHint('pub-cats-hint', '');
  setTermHint('pub-tags-hint', '');
  if (!site || !site.url) return;
  setTermHint('pub-cats-hint', '正在加载已有分类…');
  setTermHint('pub-tags-hint', '正在加载已有标签…');
  try {
    const terms = await invoke('get_terms', { site });
    if (site !== currentPubSite()) return; // 站点已切换，丢弃过期结果
    state.terms = terms;
    renderChips('pub-cats', 'pub-cats-chips', terms.categories);
    renderChips('pub-tags', 'pub-tags-chips', terms.tags);
    setTermHint('pub-cats-hint', terms.categories.length ? '' : '站点暂无已有分类');
    setTermHint('pub-tags-hint', terms.tags.length ? '' : '站点暂无已有标签');
  } catch (e) {
    // 拉取失败不阻塞发布，但要把错误展示出来
    setTermHint('pub-cats-hint', `加载失败：${e}（可手动输入）`);
    setTermHint('pub-tags-hint', `加载失败：${e}（可手动输入）`);
  }
}

function setTermHint(id, text) {
  $(id).textContent = text;
}

function currentList(inputId) {
  return $(inputId)
    .value.split(/[,，、]/)
    .map((s) => s.trim())
    .filter(Boolean);
}

const CHIP_LIMIT = 80;

function renderChips(inputId, chipsId, names) {
  const box = $(chipsId);
  box.innerHTML = '';
  if (!names.length) {
    box.hidden = true;
    return;
  }
  box.hidden = false;
  const selected = new Set(currentList(inputId));
  names.slice(0, CHIP_LIMIT).forEach((name) => {
    const chip = document.createElement('button');
    chip.type = 'button';
    chip.className = 'chip' + (selected.has(name) ? ' active' : '');
    chip.textContent = name;
    chip.addEventListener('click', () => toggleTerm(inputId, name));
    box.appendChild(chip);
  });
  if (names.length > CHIP_LIMIT) {
    const more = document.createElement('span');
    more.className = 'dim';
    more.style.alignSelf = 'center';
    more.textContent = `…共 ${names.length} 个`;
    box.appendChild(more);
  }
}

function toggleTerm(inputId, name) {
  const list = currentList(inputId);
  const idx = list.indexOf(name);
  if (idx >= 0) list.splice(idx, 1);
  else list.push(name);
  $(inputId).value = list.join('，');
  refreshChipStates(inputId);
}

function refreshChipStates(inputId) {
  const chipsId = inputId === 'pub-cats' ? 'pub-cats-chips' : 'pub-tags-chips';
  const selected = new Set(currentList(inputId));
  $(chipsId)
    .querySelectorAll('.chip')
    .forEach((c) => c.classList.toggle('active', selected.has(c.textContent)));
}

async function refreshCachedId() {
  const site = state.settings.sites[Number($('pub-site').value)];
  if (!site) return;
  const url = $('pub-slug-input').value.trim();
  try {
    const id = await invoke('get_cached_post', {
      site,
      url: url || null,
      filePath: state.filePath || 'untitled.md',
    });
    $('pub-cached-id').textContent = id || '无（将创建新文章）';
  } catch (e) {
    $('pub-cached-id').textContent = `查询失败：${e}`;
  }
}

function pubLog(text, cls = '') {
  const log = $('pub-log');
  log.hidden = false;
  const line = document.createElement('div');
  line.className = `log-line ${cls}`;
  line.textContent = text;
  log.appendChild(line);
  log.scrollTop = log.scrollHeight;
}

async function doPublish() {
  if (state.publishing) return;
  const site = state.settings.sites[Number($('pub-site').value)];
  if (!site) return;
  if (!site.url || !site.username) {
    alert('站点配置不完整，请先在设置中填写 XML-RPC 地址和用户名');
    return;
  }
  const mode = $('pub-mode').value;
  if (mode === 'manual' && !(Number($('pub-input-id').value) > 0)) {
    alert('手动模式需要填写有效的文章 ID');
    return;
  }
  const title = $('pub-title-input').value.trim();
  if (!title) {
    alert('请填写文章标题');
    return;
  }
  const splitList = (v) =>
    v
      .split(/[,，、]/)
      .map((s) => s.trim())
      .filter(Boolean);
  state.publishing = true;
  $('btn-do-publish').disabled = true;
  $('pub-log').innerHTML = '';
  pubLog(`开始发布到 ${site.name} …`, 'log-state');
  try {
    const id = await invoke('publish_post', {
      request: {
        site,
        file_path: state.filePath || 'untitled.md',
        content: $('editor').value,
        mode,
        input_id: $('pub-input-id').value,
        forced_update: $('pub-forced').checked,
        meta: {
          title,
          abstract: $('pub-abstract').value.trim() || null,
          url: $('pub-slug-input').value.trim() || null,
          date: $('pub-date').value || null,
          categories: splitList($('pub-cats').value),
          tags: splitList($('pub-tags').value),
          thumbnail: $('pub-thumb').value.trim() || null,
        },
      },
    });
    pubLog(`✔ 发布成功，文章 ID：${id}`, 'log-ok');
    setStatus(`发布成功，文章 ID：${id}`);
    await refreshCachedId();
    // 本次发布可能新建了分类/标签，刷新候选列表
    loadTerms(site);
  } catch (e) {
    pubLog(`✘ 发布失败：${e}`, 'log-error');
    setStatus('发布失败');
  } finally {
    state.publishing = false;
    $('btn-do-publish').disabled = false;
  }
}

// ---------- 新建文档 ----------

function newDoc() {
  state.filePath = null;
  state.modified = true;
  $('editor').value = '# 新文章\n\n开始写作…\n';
  $('welcome').hidden = true;
  $('split').hidden = false;
  $('btn-save').disabled = false;
  $('btn-publish').disabled = false;
  updateFileLabel();
  scheduleRender();
  setStatus('已新建文档，保存时选择文件位置');
}

// ---------- 文章管理（查/删） ----------

const mgr = { page: 1, count: 20, lastLen: 0, loading: false };

const STATUS_TEXT = {
  publish: '已发布',
  draft: '草稿',
  pending: '待审核',
  future: '定时',
  private: '私密',
  trash: '回收站',
};

function currentMgrSite() {
  return state.settings.sites[Number($('mgr-site').value)];
}

function openManager() {
  if (state.settings.sites.length === 0) {
    alert('请先在设置中添加站点');
    openSettings();
    return;
  }
  const sel = $('mgr-site');
  sel.innerHTML = '';
  state.settings.sites.forEach((site, i) => {
    const opt = document.createElement('option');
    opt.value = i;
    opt.textContent = `${site.name}（${site.username}）`;
    sel.appendChild(opt);
  });
  mgr.page = 1;
  mgr.count = Number($('mgr-count').value);
  $('manager-modal').hidden = false;
  loadPosts();
}

async function loadPosts() {
  if (mgr.loading) return;
  const site = currentMgrSite();
  if (!site) return;
  mgr.loading = true;
  $('mgr-info').textContent = '正在查询…';
  $('mgr-tbody').innerHTML = '';
  $('mgr-empty').hidden = true;
  try {
    const posts = await invoke('list_posts', {
      site,
      count: mgr.count,
      offset: (mgr.page - 1) * mgr.count,
      status: $('mgr-status').value,
      search: $('mgr-search').value,
    });
    mgr.lastLen = posts.length;
    renderPosts(posts);
    $('mgr-info').textContent = `本页 ${posts.length} 篇`;
  } catch (e) {
    $('mgr-info').textContent = `查询失败：${e}`;
    mgr.lastLen = 0;
  } finally {
    mgr.loading = false;
    updateMgrPager();
  }
}

function renderPosts(posts) {
  const tbody = $('mgr-tbody');
  tbody.innerHTML = '';
  $('mgr-empty').hidden = posts.length > 0;
  posts.forEach((p) => {
    const tr = document.createElement('tr');

    const tdId = document.createElement('td');
    tdId.textContent = p.post_id;

    const tdTitle = document.createElement('td');
    tdTitle.className = 'cell-title';
    tdTitle.textContent = p.title || '(无标题)';
    tdTitle.title = p.title;

    const tdStatus = document.createElement('td');
    const badge = document.createElement('span');
    const cls = p.status === 'publish' ? 'st-publish' : p.status === 'draft' ? 'st-draft' : 'st-other';
    badge.className = `status-badge ${cls}`;
    badge.textContent = STATUS_TEXT[p.status] || p.status || '未知';
    tdStatus.appendChild(badge);

    const tdSlug = document.createElement('td');
    tdSlug.className = 'cell-slug';
    tdSlug.textContent = p.slug;
    tdSlug.title = p.slug;

    const tdDate = document.createElement('td');
    tdDate.textContent = p.date;

    const tdOps = document.createElement('td');
    tdOps.appendChild(rowBtn('查看', () => openDetail(p.post_id)));
    tdOps.appendChild(rowBtn('更新', () => updateWithCurrent(p.post_id)));
    tdOps.appendChild(rowBtn('删除', () => confirmDelete(p), 'del'));

    tr.append(tdId, tdTitle, tdStatus, tdSlug, tdDate, tdOps);
    tbody.appendChild(tr);
  });
}

function rowBtn(text, onClick, extraCls = '') {
  const btn = document.createElement('button');
  btn.className = `row-btn ${extraCls}`;
  btn.textContent = text;
  btn.addEventListener('click', onClick);
  return btn;
}

function updateMgrPager() {
  $('mgr-page').textContent = `第 ${mgr.page} 页`;
  $('mgr-prev').disabled = mgr.page <= 1;
  $('mgr-next').disabled = mgr.lastLen < mgr.count;
}

function mgrResetAndLoad() {
  mgr.page = 1;
  loadPosts();
}

// ---------- 文章详情 ----------

async function openDetail(postId) {
  const site = currentMgrSite();
  if (!site) return;
  state.detail = null;
  $('detail-title').textContent = '文章详情';
  $('detail-meta').innerHTML = '';
  $('detail-content').innerHTML = '<p class="dim">加载中…</p>';
  $('detail-modal').hidden = false;
  try {
    const d = await invoke('get_post_detail', { site, postId });
    state.detail = d;
    $('detail-title').textContent = d.title || `文章 ${postId}`;
    renderDetailMeta(d);
    const tpl = document.createElement('template');
    tpl.innerHTML = d.html;
    tpl.content.querySelectorAll('a[href^="http"]').forEach((a) => {
      a.setAttribute('target', '_blank');
      a.setAttribute('rel', 'noopener');
    });
    $('detail-content').innerHTML = tpl.innerHTML;
  } catch (e) {
    $('detail-modal').hidden = true;
    alert(`获取文章详情失败：\n${e}`);
  }
}

function renderDetailMeta(d) {
  const rows = [
    ['ID', d.post_id],
    ['状态', STATUS_TEXT[d.status] || d.status || '未知'],
    ['固定链接', d.slug],
    ['日期', d.date],
    ['分类', d.categories.join('、') || '-'],
    ['标签', d.tags.join('、') || '-'],
    ['摘要', d.excerpt || '-'],
  ];
  const meta = $('detail-meta');
  meta.innerHTML = '';
  rows.forEach(([k, v]) => {
    const dt = document.createElement('dt');
    dt.textContent = k;
    const dd = document.createElement('dd');
    dd.textContent = v;
    meta.append(dt, dd);
  });
}

// ---------- 删除文章 ----------

async function confirmDelete(p) {
  const site = currentMgrSite();
  if (!site) return;
  const ok = await dialogConfirm(
    `确定删除文章「${p.title || p.post_id}」(ID ${p.post_id}) 吗？\n文章将移入 WordPress 回收站，本地对应的文章缓存也会被清除。`,
    { title: '删除文章', kind: 'warning' },
  );
  if (!ok) return;
  try {
    await invoke('delete_post', { site, postId: p.post_id });
    setStatus(`已删除文章（ID ${p.post_id}）`);
    if (!$('detail-modal').hidden && state.detail && state.detail.post_id === p.post_id) {
      $('detail-modal').hidden = true;
    }
    loadPosts();
  } catch (e) {
    alert(`删除失败：\n${e}`);
  }
}

// ---------- 用当前文档更新指定文章 ----------

function updateWithCurrent(postId) {
  if ($('split').hidden) {
    alert('请先打开或新建一个文档，再选择要更新的文章');
    return;
  }
  $('manager-modal').hidden = true;
  $('detail-modal').hidden = true;
  openPublish(postId);
}

// ---------- 关于与自动更新 ----------

const upd = { info: null, prepared: null, busy: false };

async function openAbout() {
  toggleAppearance(false);
  $('about-modal').hidden = false;
  try {
    const info = await invoke('get_app_info');
    $('about-version').textContent = `v${info.version}`;
    $('about-repo').textContent = info.repo;
    $('about-data-dir').textContent = info.dataDir;
    $('about-data-dir').title = info.dataDir;
  } catch (e) {
    $('about-version').textContent = '未知';
  }
}

function setUpdateStatus(text, isError = false) {
  const el = $('update-status');
  el.textContent = text;
  el.classList.toggle('error', isError);
}

async function checkForUpdate(manual = false) {
  if (upd.busy) return;
  upd.busy = true;
  $('btn-check-update').disabled = true;
  setUpdateStatus('正在检查更新…');
  try {
    const info = await invoke('check_update');
    upd.info = info;
    if (!info.hasUpdate) {
      setUpdateStatus(`已是最新版本 v${info.currentVersion}`);
      return;
    }
    setUpdateStatus(`发现新版本 v${info.latestVersion}，正在下载并校验…`);
    const prepared = await invoke('prepare_update', { info });
    upd.prepared = prepared;
    setUpdateStatus(`新版本 v${prepared.version} 已准备好`);
    $('update-version').textContent = `v${prepared.version}`;
    $('update-modal').hidden = false;
  } catch (e) {
    setUpdateStatus(`更新失败：${e}`, true);
  } finally {
    upd.busy = false;
    $('btn-check-update').disabled = false;
  }
}

async function restartForUpdate() {
  $('btn-apply-restart').disabled = true;
  $('btn-update-later').disabled = true;
  try {
    await invoke('apply_update_and_restart');
  } catch (e) {
    $('btn-apply-restart').disabled = false;
    $('btn-update-later').disabled = false;
    setUpdateStatus(`重启更新失败：${e}`, true);
  }
}

async function checkPendingUpdate() {
  try {
    const pending = await invoke('pending_update');
    if (pending) {
      $('update-version').textContent = `v${pending.version}`;
      $('update-modal').hidden = false;
    }
  } catch (e) {
    // 无暂存更新时静默
  }
}

// ---------- 事件绑定 ----------

function bindEvents() {
  $('btn-new').addEventListener('click', newDoc);
  $('btn-open').addEventListener('click', openFile);
  $('btn-welcome-open').addEventListener('click', openFile);
  $('btn-save').addEventListener('click', saveFile);
  $('btn-settings').addEventListener('click', openSettings);
  $('btn-appearance').addEventListener('click', () => toggleAppearance());
  $('btn-about').addEventListener('click', openAbout);
  $('btn-publish').addEventListener('click', () => openPublish());
  $('btn-manager').addEventListener('click', openManager);
  $('btn-add-site').addEventListener('click', () => {
    state.settings.sites.push({ name: '新站点', url: '', username: '', password: '' });
    renderSiteList();
  });
  $('btn-settings-save').addEventListener('click', saveSettings);
  $('btn-do-publish').addEventListener('click', doPublish);
  $('btn-check-update').addEventListener('click', () => checkForUpdate(true));
  $('btn-apply-restart').addEventListener('click', restartForUpdate);
  $('btn-update-later').addEventListener('click', () => {
    $('update-modal').hidden = true;
    setStatus('更新已保留，可稍后在“关于”中重启更新');
  });
  document.querySelectorAll('[data-theme-choice]').forEach((btn) => {
    btn.addEventListener('click', () => saveTheme(btn.dataset.themeChoice));
  });
  document.addEventListener('mousedown', (e) => {
    if (!$('appearance-panel').contains(e.target) && !$('btn-appearance').contains(e.target)) {
      toggleAppearance(false);
    }
  });
  systemDark.addEventListener('change', () => {
    if ((state.settings.theme || 'system') === 'system') applyTheme();
  });
  $('pub-site').addEventListener('change', () => {
    refreshCachedId();
    loadTerms(currentPubSite());
  });
  $('pub-slug-input').addEventListener('change', refreshCachedId);
  $('pub-thumb-pick').addEventListener('click', pickThumbnail);
  $('pub-cats').addEventListener('input', () => refreshChipStates('pub-cats'));
  $('pub-tags').addEventListener('input', () => refreshChipStates('pub-tags'));
  $('pub-mode').addEventListener('change', () => {
    $('row-input-id').hidden = $('pub-mode').value !== 'manual';
  });
  $('editor').addEventListener('input', scheduleRender);

  // 文章管理
  $('mgr-query').addEventListener('click', mgrResetAndLoad);
  $('mgr-site').addEventListener('change', mgrResetAndLoad);
  $('mgr-status').addEventListener('change', mgrResetAndLoad);
  $('mgr-count').addEventListener('change', () => {
    mgr.count = Number($('mgr-count').value);
    mgrResetAndLoad();
  });
  $('mgr-search').addEventListener('keydown', (e) => {
    if (e.key === 'Enter') mgrResetAndLoad();
  });
  $('mgr-prev').addEventListener('click', () => {
    if (mgr.page > 1) {
      mgr.page--;
      loadPosts();
    }
  });
  $('mgr-next').addEventListener('click', () => {
    mgr.page++;
    loadPosts();
  });
  $('detail-delete').addEventListener('click', () => {
    if (state.detail) confirmDelete(state.detail);
  });
  $('detail-update').addEventListener('click', () => {
    if (state.detail) updateWithCurrent(state.detail.post_id);
  });

  document.querySelectorAll('.close-btn').forEach((btn) => {
    btn.addEventListener('click', () => {
      $(btn.dataset.close).hidden = true;
    });
  });

  document.addEventListener('keydown', (e) => {
    if (e.ctrlKey && e.key.toLowerCase() === 's') {
      e.preventDefault();
      if (state.filePath || $('editor').value) saveFile();
    } else if (e.ctrlKey && e.key.toLowerCase() === 'o') {
      e.preventDefault();
      openFile();
    } else if (e.key === 'Escape') {
      toggleAppearance(false);
      if (!$('update-modal').hidden) $('update-modal').hidden = true;
      else if (!$('about-modal').hidden) $('about-modal').hidden = true;
      else if (!$('detail-modal').hidden) $('detail-modal').hidden = true;
      else if (!$('manager-modal').hidden) $('manager-modal').hidden = true;
      else {
        $('settings-modal').hidden = true;
        if (!state.publishing) $('publish-modal').hidden = true;
      }
    }
  });

  // 编辑器 Tab 键插入制表符
  $('editor').addEventListener('keydown', (e) => {
    if (e.key === 'Tab') {
      e.preventDefault();
      const el = e.target;
      const start = el.selectionStart;
      el.value = el.value.slice(0, start) + '\t' + el.value.slice(el.selectionEnd);
      el.selectionStart = el.selectionEnd = start + 1;
      scheduleRender();
    }
  });

  // 分隔条拖动
  const divider = $('divider');
  divider.addEventListener('mousedown', (e) => {
    e.preventDefault();
    const onMove = (ev) => {
      const pct = (ev.clientX / window.innerWidth) * 100;
      $('editor-pane').style.width = `${Math.min(80, Math.max(15, pct))}%`;
    };
    const onUp = () => {
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup', onUp);
    };
    document.addEventListener('mousemove', onMove);
    document.addEventListener('mouseup', onUp);
  });

  // 发布进度事件
  listen('publish-progress', (event) => {
    const { state: st, message } = event.payload;
    const cls = st === 'complete' ? 'log-ok' : st === 'error' ? 'log-error' : 'log-state';
    pubLog(message, cls);
    setStatus(message);
  });
}

// ---------- 启动 ----------

(async function init() {
  bindEvents();
  await loadSettings();
  applyTheme();
  setStatus('就绪');
  // 启动时静默检查更新，有新版本则下载并弹出确认
  checkForUpdate(false);
  // 之前选择“稍后”的暂存更新，启动时提示安装
  checkPendingUpdate();
})();