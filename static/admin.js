// API基础URL
const API_BASE = '';

// 当前选中的文档ID
let currentDocumentId = null;

// 分页状态
let currentPage = 0;
let pageSize = 10;
let totalDocuments = 0;

// 显示不同的页面部分
function showSection(sectionName) {
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
    
    // 设置对应导航项为active
    event.target.classList.add('active');
    
    // 根据选中的部分加载数据
    switch(sectionName) {
        case 'documents':
            loadDocuments();
            break;
        case 'preamble':
            loadPreamble();
            break;
        case 'upload':
            // 重置上传表单
            document.getElementById('createDocumentForm').reset();
            break;
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
        const response = await fetch(`${API_BASE}/api/documents?limit=${pageSize}&offset=${offset}`);
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
        const response = await fetch(`${API_BASE}/api/documents/${id}`);
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
        <div class="modal-content" style="background: white; padding: 30px; border-radius: 15px; width: 90%; max-width: 800px; max-height: 80%; overflow-y: auto; position: relative;">
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
                    <textarea id="editContent" class="form-control" rows="20" required>${escapeHtml(doc.content)}</textarea>
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
                headers: {
                    'Content-Type': 'application/json',
                },
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
        const response = await fetch(`${API_BASE}/api/documents/${id}`);
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
            <div class="modal-content" style="background: white; padding: 30px; border-radius: 15px; width: 90%; max-width: 800px; max-height: 80%; overflow-y: auto; position: relative;">
                <button class="floating-close-btn" style="position: absolute; top: 10px; right: 10px; background: rgba(0, 0, 0, 0.7); color: white; border: none; border-radius: 50%; width: 32px; height: 32px; display: flex; align-items: center; justify-content: center; cursor: pointer; z-index: 1001; opacity: 0.8; transition: opacity 0.3s ease;" title="关闭">
                    ❌
                </button>
                <h3 style="margin-bottom: 20px;">👁️ ${escapeHtml(doc.filename)}</h3>
                <div style="margin-bottom: 20px; color: #6c757d; font-size: 0.9rem;">
                    📅 创建时间: ${createdAt}<br>
                    🔄 更新时间: ${updatedAt}
                </div>
                <div style="background: #f8f9fa; padding: 20px; border-radius: 10px; margin-bottom: 20px; white-space: pre-wrap; font-family: monospace; font-size: 1rem;">${escapeHtml(doc.content)}</div>
                
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
        const response = await fetch(`${API_BASE}/api/preamble`);
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

// 处理文件选择
function handleFileSelect(event) {
    const file = event.target.files[0];
    if (!file) return;
    
    // 检查文件类型
    const allowedTypes = ['.txt', '.md', '.json', '.csv'];
    const fileExtension = '.' + file.name.split('.').pop().toLowerCase();
    
    if (!allowedTypes.includes(fileExtension)) {
        showAlert('不支持的文件类型，请选择 .txt, .md, .json 或 .csv 文件', 'error');
        return;
    }
    
    const reader = new FileReader();
    reader.onload = async function(e) {
        const content = e.target.result;
        await uploadDocument(file.name, content);
    };
    reader.readAsText(file);
}

// 上传文档
async function uploadDocument(filename, content) {
    try {
        const formData = new FormData();
        formData.append('filename', filename);
        formData.append('file', new Blob([content], { type: 'text/plain' }));
        
        const response = await fetch(`${API_BASE}/api/documents/upload`, {
            method: 'POST',
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

// 页面加载完成后初始化
document.addEventListener('DOMContentLoaded', function() {
    // 绑定Preamble表单提交
    document.getElementById('preambleForm').addEventListener('submit', async function(e) {
        e.preventDefault();
        
        const content = document.getElementById('preambleContent').value.trim();
        if (!content) {
            showAlert('请输入Preamble内容', 'error');
            return;
        }
        
        try {
            const response = await fetch(`${API_BASE}/api/preamble`, {
                method: 'PUT',
                headers: {
                    'Content-Type': 'application/json',
                },
                body: JSON.stringify({ content }),
            });
            
            if (!response.ok) {
                throw new Error('保存Preamble配置失败');
            }
            
            showAlert('Preamble配置保存成功！');
            
        } catch (error) {
            showAlert(error.message, 'error');
        }
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
                headers: {
                    'Content-Type': 'application/json',
                },
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
    
    // 初始加载文档列表
    loadDocuments();
});
