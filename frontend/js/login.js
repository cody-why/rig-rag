// Login handling
document.getElementById('loginForm').addEventListener('submit', async (e) => {
    e.preventDefault();
    
    const username = document.getElementById('username').value.trim();
    const password = document.getElementById('password').value;
    const loginBtn = document.getElementById('loginBtn');
    const alertBox = document.getElementById('alertBox');
    
    if (!username || !password) {
        showAlert('请填写用户名和密码', 'error');
        return;
    }
    
    // 禁用按钮并显示加载状态
    loginBtn.disabled = true;
    loginBtn.innerHTML = '<span class="loading-spinner"></span>登录中...';
    
    try {
        const response = await fetch('/api/auth/login', {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json',
            },
            body: JSON.stringify({ username, password }),
        });
        
        const data = await response.json();
        
        if (response.ok) {
            // 保存token到localStorage
            localStorage.setItem('authToken', data.token);
            localStorage.setItem('username', data.username);
            localStorage.setItem('userRole', data.role);
            
            showAlert('登录成功！正在跳转...', 'success');
            
            // 跳转到admin页面
            setTimeout(() => {
                window.location.href = '/admin';
            }, 500);
        } else {
            showAlert(data.error || '登录失败，请检查用户名和密码', 'error');
            loginBtn.disabled = false;
            loginBtn.innerHTML = '登录';
        }
    } catch (error) {
        console.error('Login error:', error);
        showAlert('网络错误，请稍后重试', 'error');
        loginBtn.disabled = false;
        loginBtn.innerHTML = '登录';
    }
});

// 显示提示信息
function showAlert(message, type) {
    const alertBox = document.getElementById('alertBox');
    alertBox.textContent = message;
    alertBox.className = `alert alert-${type} show`;
    
    // 3秒后自动隐藏
    setTimeout(() => {
        alertBox.classList.remove('show');
    }, 3000);
}

// 检查是否已登录
window.addEventListener('DOMContentLoaded', () => {
    const token = localStorage.getItem('authToken');
    if (token) {
        // 验证token是否有效
        fetch('/api/auth/verify', {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json',
                'Authorization': `Bearer ${token}`,
            },
        })
        .then(response => {
            if (response.ok) {
                // token有效，直接跳转到admin页面
                window.location.href = '/admin';
            }
        })
        .catch(error => {
            console.error('Token verification error:', error);
        });
    }
});

// 回车键提交表单
document.getElementById('password').addEventListener('keypress', (e) => {
    if (e.key === 'Enter') {
        e.preventDefault();
        document.getElementById('loginForm').dispatchEvent(new Event('submit'));
    }
});

