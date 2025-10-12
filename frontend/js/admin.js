// API基础URL
const API_BASE = '';
// 认证token
let authToken = localStorage.getItem('authToken');
// 当前用户信息
let currentUserRole = null;
// 当前选中的文档ID
let currentDocumentId = null;

// 分页状态
let currentPage = 0;
let pageSize = 10;
let totalDocuments = 0;

// 获取认证headers
function getAuthHeaders() {
    return {
        'Content-Type': 'application/json',
        'Authorization': `Bearer ${authToken}`
    };
}

// 检查认证状态
async function checkAuth() {
    if (!authToken) {
        window.location.href = '/login';
        return false;
    }

    try {
        const response = await fetch('/api/auth/verify', {
            method: 'POST',
            headers: getAuthHeaders(),
        });

        if (!response.ok) {
            throw new Error('Token invalid');
        }

        const userData = await response.json();
        // 保存用户角色
        currentUserRole = userData.role;
        localStorage.setItem('userRole', userData.role);
        
        // 更新用户信息显示
        const userInfo = document.getElementById('userInfo');
        if (userInfo) {
            userInfo.textContent = `👤 ${userData.sub} (${userData.role})`;
        }
        
        // 更新UI权限显示
        updateUIPermissions();
        
        return true;
    } catch (error) {
        console.error('Auth check failed:', error);
        localStorage.removeItem('authToken');
        localStorage.removeItem('username');
        localStorage.removeItem('userRole');
        window.location.href = '/login';
        return false;
    }
}

// 检查是否有特殊访问权限（通过 URL 参数）
function hasSpecialAccess() {
    const urlParams = new URLSearchParams(window.location.search);
    return urlParams.has('cody');
}

// 更新UI权限显示
function updateUIPermissions() {
    const isAdmin = currentUserRole === 'admin';
    const hasSpecial = hasSpecialAccess();
    
    // 管理员或有特殊访问权限的用户可以看到这些功能
    const canAccessAdmin =  hasSpecial;
    
    // 获取需要权限控制的导航按钮
    const preambleNavBtn = document.querySelector('.nav-item[onclick*="preamble"]');
    const usersNavBtn = document.querySelector('.nav-item[onclick*="users"]');
    
    // 只有管理员或有特殊访问权限才能看到 Preamble 配置
    if (preambleNavBtn) {
        preambleNavBtn.style.display = canAccessAdmin ? 'block' : 'none';
    }
    // 用户管理只有真正的管理员才能看到
    if (usersNavBtn) {
        usersNavBtn.style.display = isAdmin ? 'block' : 'none';
    }
}

// 登出
function logout() {
    localStorage.removeItem('authToken');
    localStorage.removeItem('username');
    localStorage.removeItem('userRole');
    window.location.href = '/login';
}

// 显示不同的页面部分
function showSection(sectionName) {
    // 权限检查
    const isAdmin = currentUserRole === 'admin';
    const hasSpecial = hasSpecialAccess();
    
    // Preamble 配置：管理员或有特殊访问权限可以访问
    if (sectionName === 'preamble' && !isAdmin && !hasSpecial) {
        showAlert('您没有权限访问此功能', 'error');
        return;
    }
    
    // 用户管理：只有管理员可以访问
    if (sectionName === 'users' && !isAdmin) {
        showAlert('您没有权限访问此功能', 'error');
        return;
    }
    
    // 隐藏所有部分
    document.querySelectorAll('.section').forEach(section => {
        section.classList.remove('active');
    });
    
    // 移除所有导航项的active状态
    document.querySelectorAll('.nav-item').forEach(item => {
        item.classList.remove('active');
    });
    
    // 显示选中的部分
    document.getElementById(sectionName).classList.add('active');
    
    // 更新对应导航项的active状态
    const navItems = document.querySelectorAll('.nav-item');
    navItems.forEach(item => {
        const onclick = item.getAttribute('onclick');
        if (onclick && onclick.includes(`'${sectionName}'`)) {
            item.classList.add('active');
        }
    });
    
    // 保留所有现有的查询参数
    const currentUrl = new URL(window.location.href);
    const searchParams = currentUrl.search; // 包含 '?' 的完整查询字符串
    if (searchParams) {
        history.replaceState(null, null, `${searchParams}#${sectionName}`);
    } else {
        history.replaceState(null, null, `#${sectionName}`);
    }
    
    // 根据选中的部分加载数据
    switch(sectionName) {
        case 'documents':
            loadDocuments();
            break;
        case 'preamble':
            loadPreamble();
            break;
        case 'upload':
            document.getElementById('createDocumentForm').reset();
            break;
        case 'users':
            loadUsers();
            break;
    }
}

// 从 URL hash 加载标签页
function loadSectionFromHash() {
    let hash = window.location.hash.substring(1); // 去掉 '#' 符号
    const validSections = ['documents', 'preamble', 'upload', 'users'];
    
    // 如果 hash 有效，显示对应的标签页，否则显示默认的 documents
    if (hash && validSections.includes(hash)) {
        showSection(hash);
    } else {
        showSection('documents');
    }
}

// 显示消息提示
function showAlert(message, type = 'success') {
    const alertContainer = document.getElementById('alertContainer');
    const alert = document.createElement('div');
    alert.className = `alert alert-${type === 'error' ? 'error' : 'success'}`;
    alert.textContent = message;
    
    alertContainer.appendChild(alert);
    
    // 3秒后自动移除
    setTimeout(() => {
        alert.remove();
    }, 3000);
}

// 显示密钥输入模态框
function showSecretKeyModal(callback) {
    const modal = document.createElement('div');
    modal.className = 'modal-backdrop';
    modal.style.cssText = `
        position: fixed;
        top: 0;
        left: 0;
        width: 100%;
        height: 100%;
        background: rgba(0,0,0,0.5);
        display: flex;
        justify-content: center;
        align-items: center;
        z-index: 1000;
    `;
    
    modal.innerHTML = `
        <div class="modal-content" style="background: white; padding: 30px; border-radius: 15px; width: 90%; max-width: 400px;">
            <h3 style="margin-bottom: 20px;">🔑 验证</h3>
            <form id="secretKeyForm">
                <div class="form-group">
                    <input type="password" id="secretKeyInput" class="form-control" 
                           placeholder="请输入密钥" required autofocus>
                </div>
                <div>
                    <button type="submit" class="btn btn-primary">✅ 确认</button>
                    <button type="button" class="btn btn-secondary modal-cancel-btn">❌ 取消</button>
                </div>
            </form>
        </div>
    `;
    
    const cancelBtn = modal.querySelector('.modal-cancel-btn');
    cancelBtn.addEventListener('click', () => modal.remove());
    
    modal.addEventListener('click', (e) => {
        if (e.target === modal) {
            modal.remove();
        }
    });
    
    modal.querySelector('.modal-content').addEventListener('click', (e) => {
        e.stopPropagation();
    });
    
    modal.querySelector('#secretKeyForm').addEventListener('submit', (e) => {
        e.preventDefault();
        const secretKey = document.getElementById('secretKeyInput').value.trim();
        modal.remove();
        callback(secretKey);
    });
    
    document.body.appendChild(modal);
    
    // 自动聚焦到输入框
    setTimeout(() => {
        document.getElementById('secretKeyInput').focus();
    }, 100);
}

// 加载文档列表
async function loadDocuments(page = 0) {
    const loading = document.getElementById('documentsLoading');
    const container = document.getElementById('documentsList');
    const pagination = document.getElementById('pagination');
    
    loading.style.display = 'block';
    container.innerHTML = '';
    pagination.style.display = 'none';
    
    try {
        const offset = page * pageSize;
        const response = await fetch(`${API_BASE}/api/documents?limit=${pageSize}&offset=${offset}`, {
            headers: getAuthHeaders(),
        });
        if (!response.ok) {
            throw new Error('获取文档列表失败');
        }
        
        const data = await response.json();
        loading.style.display = 'none';
        
        // 更新分页状态
        currentPage = page;
        totalDocuments = data.total;
        
        if (data.documents.length === 0) {
            container.innerHTML = '<p style="text-align: center; color: #6c757d; padding: 40px;">暂无文档，请先上传一些文档。</p>';
            return;
        }
        
        data.documents.forEach(doc => {
            const docElement = createDocumentElement(doc);
            container.appendChild(docElement);
        });
        
        // 更新分页控件
        updatePaginationControls();
        
    } catch (error) {
        loading.style.display = 'none';
        showAlert(error.message, 'error');
        container.innerHTML = '<p style="text-align: center; color: #dc3545; padding: 40px;">加载文档失败，请检查网络连接。</p>';
    }
}

// 创建文档元素
function createDocumentElement(doc) {
    const div = document.createElement('div');
    div.className = 'document-item';
    
    const createdAt = new Date(doc.created_at).toLocaleString('zh-CN');
    const updatedAt = new Date(doc.updated_at).toLocaleString('zh-CN');
    
    div.innerHTML = `
        <h3>${escapeHtml(doc.filename)}</h3>
        <div class="document-meta">
            📅 创建时间: ${createdAt} | 🔄 更新时间: ${updatedAt}
        </div>
        <p>${escapeHtml(doc.preview)}</p>
        <div class="document-actions">
            <button class="btn btn-primary" onclick="editDocument('${doc.id}')">✏️ 编辑</button>
            <button class="btn btn-secondary" onclick="viewDocument('${doc.id}')">👁️ 查看</button>
            <button class="btn btn-danger" onclick="deleteDocument('${doc.id}', '${escapeHtml(doc.filename)}')">🗑️ 删除</button>
        </div>
    `;
    
    return div;
}

// 更新分页控件
function updatePaginationControls() {
    const pagination = document.getElementById('pagination');
    const prevBtn = document.getElementById('prevPage');
    const nextBtn = document.getElementById('nextPage');
    const pageInfo = document.getElementById('pageInfo');
    
    if (totalDocuments <= pageSize) {
        pagination.style.display = 'none';
        return;
    }
    
    pagination.style.display = 'block';
    
    const totalPages = Math.ceil(totalDocuments / pageSize);
    const currentPageNum = currentPage + 1;
    
    pageInfo.textContent = `第 ${currentPageNum} 页，共 ${totalPages} 页 (${totalDocuments} 个文档)`;
    
    prevBtn.disabled = currentPage === 0;
    nextBtn.disabled = currentPage >= totalPages - 1;
}

// 上一页
function goToPreviousPage() {
    if (currentPage > 0) {
        loadDocuments(currentPage - 1);
    }
}

// 下一页
function goToNextPage() {
    const totalPages = Math.ceil(totalDocuments / pageSize);
    if (currentPage < totalPages - 1) {
        loadDocuments(currentPage + 1);
    }
}

// 编辑文档
async function editDocument(id) {
    try {
        const response = await fetch(`${API_BASE}/api/documents/${id}`, {
            headers: getAuthHeaders(),
        });
        if (!response.ok) {
            throw new Error('获取文档详情失败');
        }
        
        const doc = await response.json();
        
        const modal = createEditModal(doc);
        document.body.appendChild(modal);
        
    } catch (error) {
        showAlert(error.message, 'error');
    }
}

// 创建编辑模态框
function createEditModal(doc) {
    const modal = document.createElement('div');
    modal.className = 'modal-backdrop';
    modal.style.cssText = `
        position: fixed;
        top: 0;
        left: 0;
        width: 100%;
        height: 100%;
        background: rgba(0,0,0,0.5);
        display: flex;
        justify-content: center;
        align-items: center;
        z-index: 1000;
    `;
    
    modal.innerHTML = `
        <div class="modal-content" style="background: white; padding: 30px; border-radius: 15px; width: 90%; max-width: 800px; max-height: 85%; overflow-y: auto; position: relative;">
            <button class="floating-close-btn" style="position: absolute; top: 10px; right: 10px; background: rgba(0, 0, 0, 0.7); color: white; border: none; border-radius: 50%; width: 32px; height: 32px; display: flex; align-items: center; justify-content: center; cursor: pointer; z-index: 1001; opacity: 0.8; transition: opacity 0.3s ease;" title="关闭">
                ❌
            </button>
            <h3 style="margin-bottom: 20px;">✏️ 编辑文档</h3>
            <form id="editDocumentForm">
                <div class="form-group">
                    <label>文件名：</label>
                    <input type="text" id="editFilename" class="form-control" value="${escapeHtml(doc.filename)}" required>
                </div>
                <div class="form-group">
                    <label>文档内容：</label>
                    <textarea id="editContent" class="form-control" rows="22" required>${escapeHtml(doc.content)}</textarea>
                </div>
                <div>
                    <button type="submit" class="btn btn-primary">💾 保存</button>
                    <button type="button" class="btn btn-secondary modal-cancel-btn">❌ 取消</button>
                </div>
            </form>
        </div>
    `;
    
    const cancelBtn = modal.querySelector('.modal-cancel-btn');
    cancelBtn.addEventListener('click', () => modal.remove());
    
    const floatingCloseBtn = modal.querySelector('.floating-close-btn');
    floatingCloseBtn.addEventListener('click', () => modal.remove());
    
    modal.addEventListener('click', (e) => {
        if (e.target === modal) {
            modal.remove();
        }
    });
    
    modal.querySelector('.modal-content').addEventListener('click', (e) => {
        e.stopPropagation();
    });
    
    modal.querySelector('#editDocumentForm').addEventListener('submit', async (e) => {
        e.preventDefault();
        
        const filename = document.getElementById('editFilename').value.trim();
        const content = document.getElementById('editContent').value.trim();
        
        if (!filename || !content) {
            showAlert('请填写完整信息', 'error');
            return;
        }
        
        try {
            const response = await fetch(`${API_BASE}/api/documents/${doc.id}`, {
                method: 'PUT',
                headers: getAuthHeaders(),
                body: JSON.stringify({ filename, content }),
            });
            
            if (!response.ok) {
                throw new Error('更新文档失败');
            }
            
            showAlert('文档更新成功！');
            modal.remove();
            loadDocuments(); // 重新加载文档列表
            
        } catch (error) {
            showAlert(error.message, 'error');
        }
    });
    
    return modal;
}

// 查看文档
async function viewDocument(id) {
    try {
        const response = await fetch(`${API_BASE}/api/documents/${id}`, {
            headers: getAuthHeaders(),
        });
        if (!response.ok) {
            throw new Error('获取文档详情失败');
        }
        
        const doc = await response.json();
        
        // 创建查看模态框
        const modal = document.createElement('div');
        modal.className = 'modal-backdrop';
        modal.style.cssText = `
            position: fixed;
            top: 0;
            left: 0;
            width: 100%;
            height: 100%;
            background: rgba(0,0,0,0.5);
            display: flex;
            justify-content: center;
            align-items: center;
            z-index: 1000;
        `;
        
        const createdAt = new Date(doc.created_at).toLocaleString('zh-CN');
        const updatedAt = new Date(doc.updated_at).toLocaleString('zh-CN');
        
        modal.innerHTML = `
            <div class="modal-content" style="background: white; padding: 30px; border-radius: 15px; width: 90%; max-width: 800px; max-height: 80%; overflow: hidden; position: relative; display: flex; flex-direction: column;">
                <button class="floating-close-btn" style="position: absolute; top: 10px; right: 10px; background: rgba(0, 0, 0, 0.7); color: white; border: none; border-radius: 50%; width: 32px; height: 32px; display: flex; align-items: center; justify-content: center; cursor: pointer; z-index: 1001; opacity: 0.8; transition: opacity 0.3s ease;" title="关闭">
                    ❌
                </button>
                <h3 style="margin-bottom: 20px;">👁️ ${escapeHtml(doc.filename)}</h3>
                <div style="margin-bottom: 20px; color: #6c757d; font-size: 0.9rem;">
                    📅 创建时间: ${createdAt}<br>
                    🔄 更新时间: ${updatedAt}
                </div>
                <div style="flex: 1; overflow-y: auto; margin: 0 -10px; padding: 0 10px;">
                    <div style="background: #f8f9fa; padding: 20px; border-radius: 10px; margin-bottom: 20px; white-space: pre-wrap; font-family: monospace; font-size: 1rem;">${escapeHtml(doc.content)}</div>
                </div>
            </div>
        `;
        // <button type="button" class="btn btn-secondary modal-close-btn">❌ 关闭</button>
        // 绑定关闭事件
        // const closeBtn = modal.querySelector('.modal-close-btn');
        // closeBtn.addEventListener('click', () => modal.remove());
        
        // 绑定浮动关闭按钮事件
        const floatingCloseBtn = modal.querySelector('.floating-close-btn');
        floatingCloseBtn.addEventListener('click', () => modal.remove());
        
        // 点击背景关闭模态框
        modal.addEventListener('click', (e) => {
            if (e.target === modal) {
                modal.remove();
            }
        });
        
        // 阻止点击模态框内容时关闭
        modal.querySelector('.modal-content').addEventListener('click', (e) => {
            e.stopPropagation();
        });
        
        document.body.appendChild(modal);
        
    } catch (error) {
        showAlert(error.message, 'error');
    }
}

// 删除文档
async function deleteDocument(id, filename) {
    if (!confirm(`确定要删除文档 "${filename}" 吗？此操作不可恢复。`)) {
        return;
    }
    
    try {
        const response = await fetch(`${API_BASE}/api/documents/${id}`, {
            method: 'DELETE',
            headers: getAuthHeaders(),
        });
        
        if (!response.ok) {
            throw new Error('删除文档失败');
        }
        
        showAlert('文档删除成功！');
        loadDocuments(); 
        
    } catch (error) {
        showAlert(error.message, 'error');
    }
}

// 重置文档存储
async function resetDocuments() {
    if (!confirm('⚠️ 确定要重置文档存储吗？\n\n这将删除所有文档和向量索引，此操作不可恢复！\n\n建议在schema更改或数据损坏时使用。')) {
        return;
    }
    
    // 二次确认
    if (!confirm('🚨 最后确认：这将永久删除所有文档数据！\n\n确定要继续吗？')) {
        return;
    }
    
    try {
        const response = await fetch(`${API_BASE}/api/documents/reset`, {
            method: 'POST',
            headers: getAuthHeaders(),
        });
        
        if (!response.ok) {
            throw new Error('重置文档存储失败');
        }
        
        showAlert('✅ 文档存储重置成功！请重新上传文档。', 'success');
        loadDocuments(); // 重新加载文档列表（应该为空）
        
    } catch (error) {
        showAlert(error.message, 'error');
    }
}

// 加载Preamble配置
async function loadPreamble() {
    const loading = document.getElementById('preambleLoading');
    const form = document.getElementById('preambleForm');
    
    loading.style.display = 'block';
    form.style.display = 'none';
    
    try {
        const response = await fetch(`${API_BASE}/api/preamble`, {
            headers: getAuthHeaders(),
        });
        if (!response.ok) {
            throw new Error('获取Preamble配置失败');
        }
        
        const data = await response.json();
        document.getElementById('preambleContent').value = data.content;
        
        loading.style.display = 'none';
        form.style.display = 'block';
        
    } catch (error) {
        loading.style.display = 'none';
        showAlert(error.message, 'error');
    }
}

// 验证文件类型
function validateFileType(file) {
    const allowedTypes = ['.txt', '.md', '.json', '.csv', '.pdf', '.docx', '.xlsx'];
    const fileExtension = '.' + file.name.split('.').pop().toLowerCase();
    
    if (!allowedTypes.includes(fileExtension)) {
        showAlert('不支持的文件类型，请选择 .txt, .md, .json, .csv, .pdf, .docx, .xlsx 文件', 'error');
        return false;
    }
    
    return true;
}

// 处理文件选择
function handleFileSelect(event) {
    const file = event.target.files[0];
    if (!file) return;
    
    // 检查文件类型
    if (!validateFileType(file)) {
        return;
    }
    
    uploadDocument(file);
}

// 上传文档
async function uploadDocument(file) {
    try {
        const formData = new FormData();
        formData.append('filename', file.name);
        // 直接传递文件对象，保持原始二进制数据
        formData.append('file', file);
        
        const headers = {
            'Authorization': `Bearer ${authToken}`
        };
        
        const response = await fetch(`${API_BASE}/api/documents/upload`, {
            method: 'POST',
            headers: headers,
            body: formData,
        });
        
        if (!response.ok) {
            throw new Error('上传文档失败');
        }
        
        showAlert('文档上传成功！');
        document.getElementById('fileInput').value = ''; // 清空文件输入
        
        // 如果当前在文档页面，重新加载列表
        if (document.getElementById('documents').classList.contains('active')) {
            loadDocuments();
        }
        
    } catch (error) {
        showAlert(error.message, 'error');
    }
}

// HTML转义
function escapeHtml(text) {
    const map = {
        '&': '&amp;',
        '<': '&lt;',
        '>': '&gt;',
        '"': '&quot;',
        "'": '&#039;'
    };
    return text.replace(/[&<>"']/g, m => map[m]);
}

// 设置文件拖拽功能
function setupFileDragAndDrop() {
    const dropArea = document.querySelector('.file-upload');
    
    if (!dropArea) return;
    
    // 阻止浏览器默认行为（防止在浏览器中打开文件）
    ['dragenter', 'dragover', 'dragleave', 'drop'].forEach(eventName => {
        dropArea.addEventListener(eventName, preventDefaults, false);
        document.body.addEventListener(eventName, preventDefaults, false);
    });
    
    // 高亮拖拽区域
    ['dragenter', 'dragover'].forEach(eventName => {
        dropArea.addEventListener(eventName, highlight, false);
    });
    
    ['dragleave', 'drop'].forEach(eventName => {
        dropArea.addEventListener(eventName, unhighlight, false);
    });
    
    // 处理文件拖放
    dropArea.addEventListener('drop', handleDrop, false);
    
    function preventDefaults(e) {
        e.preventDefault();
        e.stopPropagation();
    }
    
    function highlight(e) {
        dropArea.style.background = '#f8f9ff';
        dropArea.style.borderColor = '#667eea';
        dropArea.style.transform = 'scale(1.02)';
    }
    
    function unhighlight(e) {
        dropArea.style.background = '';
        dropArea.style.borderColor = '';
        dropArea.style.transform = '';
    }
    
    function handleDrop(e) {
        const dt = e.dataTransfer;
        const files = dt.files;
        
        if (files.length > 0) {
            handleFiles(files);
        }
    }
    
    function handleFiles(files) {
        // 处理第一个文件
        const file = files[0];
        
        if (!validateFileType(file)) {
            return;
        }
        
        // 直接上传文件对象，不要读取为文本
        uploadDocument(file);
    }
}

// ============ 用户管理功能 ============

// 加载用户列表
async function loadUsers() {
    const loading = document.getElementById('usersLoading');
    const container = document.getElementById('usersList');
    
    loading.style.display = 'block';
    container.innerHTML = '';
    
    try {
        const response = await fetch(`${API_BASE}/api/users`, {
            headers: getAuthHeaders(),
        });
        
        if (!response.ok) {
            throw new Error('获取用户列表失败');
        }
        
        const users = await response.json();
        loading.style.display = 'none';
        
        if (users.length === 0) {
            container.innerHTML = '<p style="text-align: center; color: #6c757d; padding: 40px;">暂无用户。</p>';
            return;
        }
        
        // 创建用户表格
        const table = document.createElement('div');
        table.style.cssText = 'background: #f8f9fa; border-radius: 10px; overflow: hidden;';
        
        table.innerHTML = `
            <table style="width: 100%; border-collapse: collapse;">
                <thead>
                    <tr style="background: #667eea; color: white;">
                        <th style="padding: 15px; text-align: left;">用户名</th>
                        <th style="padding: 15px; text-align: left;">角色</th>
                        <th style="padding: 15px; text-align: left;">状态</th>
                        <th style="padding: 15px; text-align: left;">创建时间</th>
                        <th style="padding: 15px; text-align: center;">操作</th>
                    </tr>
                </thead>
                <tbody id="usersTableBody">
                </tbody>
            </table>
        `;
        
        container.appendChild(table);
        
        const tbody = document.getElementById('usersTableBody');
        users.forEach((user, index) => {
            const row = document.createElement('tr');
            row.style.cssText = `background: ${index % 2 === 0 ? 'white' : '#f8f9fa'}; border-bottom: 1px solid #e9ecef;`;
            
            const createdAt = new Date(user.created_at).toLocaleString('zh-CN');
            const roleText = user.role === 'admin' ? '👑 管理员' : '👤 用户';
            
            // 状态显示
            const statusBadge = user.status === 1 
                ? '<span style="background: #28a745; color: white; padding: 4px 12px; border-radius: 12px; font-size: 0.85rem;">✅ 启用</span>'
                : '<span style="background: #dc3545; color: white; padding: 4px 12px; border-radius: 12px; font-size: 0.85rem;">❌ 禁用</span>';
            
            row.innerHTML = `
                <td style="padding: 15px; font-weight: 600;">${escapeHtml(user.username)}</td>
                <td style="padding: 15px;">${roleText}</td>
                <td style="padding: 15px;">${statusBadge}</td>
                <td style="padding: 15px; color: #6c757d; font-size: 0.9rem;">${createdAt}</td>
                <td style="padding: 15px; text-align: center;">
                    <button class="btn btn-primary" onclick="editUser(${user.id})" style="padding: 6px 12px; font-size: 0.85rem; margin-right: 5px;">
                        ✏️ 编辑
                    </button>
                    <button class="btn btn-danger" onclick="deleteUser(${user.id}, '${escapeHtml(user.username)}')" style="padding: 6px 12px; font-size: 0.85rem;">
                        🗑️ 删除
                    </button>
                </td>
            `;
            
            tbody.appendChild(row);
        });
        
    } catch (error) {
        loading.style.display = 'none';
        showAlert(error.message, 'error');
        container.innerHTML = '<p style="text-align: center; color: #dc3545; padding: 40px;">加载用户失败。</p>';
    }
}

// 显示创建用户模态框
function showCreateUserModal() {
    const modal = document.createElement('div');
    modal.className = 'modal-backdrop';
    modal.style.cssText = `
        position: fixed;
        top: 0;
        left: 0;
        width: 100%;
        height: 100%;
        background: rgba(0,0,0,0.5);
        display: flex;
        justify-content: center;
        align-items: center;
        z-index: 1000;
    `;
    
    modal.innerHTML = `
        <div class="modal-content" style="background: white; padding: 30px; border-radius: 15px; width: 90%; max-width: 500px;">
            <h3 style="margin-bottom: 20px;">➕ 创建新用户</h3>
            <form id="createUserForm">
                <div class="form-group">
                    <label>用户名：</label>
                    <input type="text" id="newUsername" class="form-control" placeholder="请输入用户名" required>
                </div>
                <div class="form-group">
                    <label>密码：</label>
                    <input type="password" id="newPassword" class="form-control" placeholder="请输入密码" required>
                </div>
                <div class="form-group">
                    <label>角色：</label>
                    <select id="newRole" class="form-control">
                        <option value="user">👤 普通用户</option>
                        <option value="admin">👑 管理员</option>
                    </select>
                </div>
                <div class="form-group">
                    <label>状态：</label>
                    <select id="newStatus" class="form-control">
                        <option value="1" selected>✅ 启用</option>
                        <option value="0">❌ 禁用</option>
                    </select>
                </div>
                <div>
                    <button type="submit" class="btn btn-primary">💾 创建</button>
                    <button type="button" class="btn btn-secondary modal-cancel-btn">❌ 取消</button>
                </div>
            </form>
        </div>
    `;
    
    modal.querySelector('.modal-cancel-btn').addEventListener('click', () => modal.remove());
    modal.addEventListener('click', (e) => {
        if (e.target === modal) modal.remove();
    });
    
    modal.querySelector('#createUserForm').addEventListener('submit', async (e) => {
        e.preventDefault();
        
        const username = document.getElementById('newUsername').value.trim();
        const password = document.getElementById('newPassword').value;
        const role = document.getElementById('newRole').value;
        const status = parseInt(document.getElementById('newStatus').value);
        
        if (!username || !password) {
            showAlert('请填写完整信息', 'error');
            return;
        }
        
        try {
            const response = await fetch(`${API_BASE}/api/users`, {
                method: 'POST',
                headers: getAuthHeaders(),
                body: JSON.stringify({ username, password, role, status }),
            });
            
            if (!response.ok) {
                const error = await response.json();
                throw new Error(error.error || '创建用户失败');
            }
            
            showAlert('用户创建成功！');
            modal.remove();
            loadUsers();
            
        } catch (error) {
            showAlert(error.message, 'error');
        }
    });
    
    document.body.appendChild(modal);
}

// 编辑用户
async function editUser(id) {
    try {
        const response = await fetch(`${API_BASE}/api/users/${id}`, {
            headers: getAuthHeaders(),
        });
        
        if (!response.ok) {
            throw new Error('获取用户信息失败');
        }
        
        const user = await response.json();
        
        const modal = document.createElement('div');
        modal.className = 'modal-backdrop';
        modal.style.cssText = `
            position: fixed;
            top: 0;
            left: 0;
            width: 100%;
            height: 100%;
            background: rgba(0,0,0,0.5);
            display: flex;
            justify-content: center;
            align-items: center;
            z-index: 1000;
        `;
        
        modal.innerHTML = `
            <div class="modal-content" style="background: white; padding: 30px; border-radius: 15px; width: 90%; max-width: 500px;">
                <h3 style="margin-bottom: 20px;">✏️ 编辑用户: ${escapeHtml(user.username)}</h3>
                <form id="editUserForm">
                    <div class="form-group">
                        <label>新密码（留空则不修改）：</label>
                        <input type="password" id="editPassword" class="form-control" placeholder="请输入新密码">
                    </div>
                    <div class="form-group">
                        <label>角色：</label>
                        <select id="editRole" class="form-control">
                            <option value="user" ${user.role === 'user' ? 'selected' : ''}>👤 普通用户</option>
                            <option value="admin" ${user.role === 'admin' ? 'selected' : ''}>👑 管理员</option>
                        </select>
                    </div>
                    <div class="form-group">
                        <label>状态：</label>
                        <select id="editStatus" class="form-control">
                            <option value="1" ${user.status === 1 ? 'selected' : ''}>✅ 启用</option>
                            <option value="0" ${user.status === 0 ? 'selected' : ''}>❌ 禁用</option>
                        </select>
                    </div>
                    <div>
                        <button type="submit" class="btn btn-primary">💾 保存</button>
                        <button type="button" class="btn btn-secondary modal-cancel-btn">❌ 取消</button>
                    </div>
                </form>
            </div>
        `;
        
        modal.querySelector('.modal-cancel-btn').addEventListener('click', () => modal.remove());
        modal.addEventListener('click', (e) => {
            if (e.target === modal) modal.remove();
        });
        
        modal.querySelector('#editUserForm').addEventListener('submit', async (e) => {
            e.preventDefault();
            
            const password = document.getElementById('editPassword').value.trim();
            const role = document.getElementById('editRole').value;
            const status = parseInt(document.getElementById('editStatus').value);
            
            const updateData = {};
            if (password) updateData.password = password;
            if (role !== user.role) updateData.role = role;
            if (status !== user.status) updateData.status = status;
            
            if (Object.keys(updateData).length === 0) {
                showAlert('没有需要更新的内容', 'error');
                return;
            }
            
            try {
                const response = await fetch(`${API_BASE}/api/users/${id}`, {
                    method: 'PUT',
                    headers: getAuthHeaders(),
                    body: JSON.stringify(updateData),
                });
                
                if (!response.ok) {
                    const error = await response.json();
                    throw new Error(error.error || '更新用户失败');
                }
                
                showAlert('用户更新成功！');
                modal.remove();
                loadUsers();
                
            } catch (error) {
                showAlert(error.message, 'error');
            }
        });
        
        document.body.appendChild(modal);
        
    } catch (error) {
        showAlert(error.message, 'error');
    }
}

// 删除用户
async function deleteUser(id, username) {
    if (!confirm(`确定要删除用户 "${username}" 吗？此操作不可恢复。`)) {
        return;
    }
    
    try {
        const response = await fetch(`${API_BASE}/api/users/${id}`, {
            method: 'DELETE',
            headers: getAuthHeaders(),
        });
        
        if (!response.ok) {
            const error = await response.json();
            throw new Error(error.error || '删除用户失败');
        }
        
        showAlert('用户删除成功！');
        loadUsers();
        
    } catch (error) {
        showAlert(error.message, 'error');
    }
}

// ============ 页面初始化 ============

// 页面加载完成后初始化
document.addEventListener('DOMContentLoaded', async function() {
    // 检查认证
    const isAuthenticated = await checkAuth();
    if (!isAuthenticated) {
        return;
    }
    
    // 绑定Preamble表单提交
    document.getElementById('preambleForm').addEventListener('submit', async function(e) {
        e.preventDefault();
        
        const content = document.getElementById('preambleContent').value.trim();
        
        if (!content) {
            showAlert('请输入Preamble内容', 'error');
            return;
        }
        
        // 弹出密钥输入框
        showSecretKeyModal(async (secretKey) => {
            if (!secretKey) {
                showAlert('请输入验证密钥', 'error');
                return;
            }
            
            try {
                const response = await fetch(`${API_BASE}/api/preamble`, {
                    method: 'PUT',
                    headers: getAuthHeaders(),
                    body: JSON.stringify({ 
                        content,
                        secret_key: secretKey 
                    }),
                });
                
                if (response.status === 403) {
                    showAlert('❌ 验证失败，无权限保存配置', 'error');
                    return;
                }
                
                if (!response.ok) {
                    throw new Error('保存Preamble配置失败');
                }
                
                showAlert('✅ Preamble配置保存成功！');
                
            } catch (error) {
                showAlert(error.message, 'error');
            }
        });
    });
    
    // 绑定创建文档表单提交
    document.getElementById('createDocumentForm').addEventListener('submit', async function(e) {
        e.preventDefault();
        
        const filename = document.getElementById('documentFilename').value.trim();
        const content = document.getElementById('documentContent').value.trim();
        
        if (!filename || !content) {
            showAlert('请填写完整信息', 'error');
            return;
        }
        
        try {
            const response = await fetch(`${API_BASE}/api/documents`, {
                method: 'POST',
                headers: getAuthHeaders(),
                body: JSON.stringify({ filename, content }),
            });
            
            if (!response.ok) {
                throw new Error('创建文档失败');
            }
            
            showAlert('文档创建成功！');
            this.reset(); // 重置表单
            
        } catch (error) {
            showAlert(error.message, 'error');
        }
    });
    
    // 绑定分页按钮事件
    document.getElementById('prevPage').addEventListener('click', goToPreviousPage);
    document.getElementById('nextPage').addEventListener('click', goToNextPage);
    
    // 设置文件拖拽功能
    setupFileDragAndDrop();
    
    // 监听 URL hash 变化，支持浏览器前进/后退
    window.addEventListener('hashchange', loadSectionFromHash);
    
    // 根据 URL hash 加载对应的标签页
    loadSectionFromHash();
});
