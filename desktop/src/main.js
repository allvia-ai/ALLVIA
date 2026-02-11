import './style.css'

// Agent API URL
const API_BASE = 'http://localhost:5680/api';

document.querySelector('#app').innerHTML = `
  <header class="header">
    <div class="logo">
      <div class="logo-icon">🤖</div>
      <span class="logo-text">Steer Agent</span>
    </div>
    <div class="status-badge" id="statusBadge">
      <span class="status-dot"></span>
      <span id="statusText">Connecting...</span>
    </div>
  </header>

  <main class="main-content">
    <section class="chat-section">
      <div class="chat-messages" id="messages">
        <div class="message agent">
          <span class="emoji">👋</span> 안녕하세요! 무엇을 도와드릴까요?
          <br><br>
          자연어로 말씀해주세요:
          <br>• "이메일 보여줘"
          <br>• "오늘 일정 뭐야?"
          <br>• "시스템 상태 알려줘"
        </div>
      </div>
      <div class="input-section">
        <div class="input-wrapper">
          <input 
            type="text" 
            class="chat-input" 
            id="userInput"
            placeholder="자연어로 명령을 입력하세요..."
            autocomplete="off"
          />
        </div>
        <button class="send-btn" id="sendBtn">전송</button>
      </div>
    </section>

    <aside class="sidebar">
      <div class="card">
        <div class="card-title">빠른 실행</div>
        <div class="quick-actions">
          <button class="action-btn" data-cmd="이메일 보여줘">
            <span class="icon">📧</span> 이메일 확인
          </button>
          <button class="action-btn" data-cmd="오늘 일정 뭐야?">
            <span class="icon">📅</span> 오늘 일정
          </button>
          <button class="action-btn" data-cmd="시스템 상태 알려줘">
            <span class="icon">📊</span> 시스템 상태
          </button>
          <button class="action-btn" data-cmd="analyze_patterns">
            <span class="icon">🔍</span> 패턴 분석
          </button>
        </div>
      </div>

      <div class="card">
        <div class="card-title">시스템</div>
        <div class="system-stats">
          <div>
            <div class="stat-row">
              <span class="stat-label">CPU</span>
              <span class="stat-value" id="cpuValue">--</span>
            </div>
            <div class="stat-bar">
              <div class="stat-bar-fill" id="cpuBar" style="width: 0%"></div>
            </div>
          </div>
          <div>
            <div class="stat-row">
              <span class="stat-label">RAM</span>
              <span class="stat-value" id="ramValue">--</span>
            </div>
            <div class="stat-bar">
              <div class="stat-bar-fill" id="ramBar" style="width: 0%"></div>
            </div>
          </div>
        </div>
      </div>

      <div class="card">
        <div class="card-title">내 루틴 (My Routines)</div>
        <div id="routinesList" class="routines-list">
          <div class="loading">로딩 중...</div>
        </div>
      </div>

      <div class="card">
        <div class="card-title">추천 워크플로우</div>
        <div class="recommendations" id="recsContainer">
          <div class="loading">로딩 중...</div>
        </div>
      </div>

      <!-- ... Integrations ... -->

// ... JS Code ...

// Fetch Routines
async function fetchRoutines() {
  const container = document.getElementById('routinesList');
  try {
    const resp = await fetch(`${ API_BASE }/routines`);
if (resp.ok) {
  const routines = await resp.json();
  if (routines.length === 0) {
    container.innerHTML = '<div class="empty-state">아직 등록된 루틴이 없습니다.<br><small style="color:#666">"매일 아침 9시 뉴스해줘"라고 말해보세요!</small></div>';
  } else {
    container.innerHTML = routines.map(r => {
      const nextRun = r.next_run ? new Date(r.next_run).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : 'Pending';
      return `
          <div class="routine-item" style="margin-bottom:8px; padding:8px; background:rgba(0,0,0,0.2); border-radius:4px;">
            <div style="display:flex; justify-content:space-between; align-items:center;">
                <span style="font-weight:600; font-size:0.9rem;">${r.name}</span>
                <span style="font-size:0.75rem; color:${r.enabled ? '#4caf50' : '#888'}">${r.enabled ? 'ON' : 'OFF'}</span>
            </div>
            <div style="font-size:0.8rem; color:#aaa; margin-top:2px;">${r.cron_expression}</div>
            <div style="font-size:0.75rem; color:#666; margin-top:4px;">🔜 Next: ${nextRun}</div>
          </div>
        `}).join('');
  }
}
  } catch (e) {
  console.error(e);
}
}

// Check API health
async function checkHealth() {
  // ... existing code ...

  // In fetchStatus or interval, add fetchRoutines()
  setInterval(fetchRoutines, 10000);
  fetchRoutines();

  <div class="card">
    <div class="card-title">연동 서비스</div>
    <div class="integrations">
      <div class="integration-badge">
        <span class="integration-dot"></span> Gmail
      </div>
      <div class="integration-badge">
        <span class="integration-dot"></span> Calendar
      </div>
      <div class="integration-badge">
        <span class="integration-dot"></span> Telegram
      </div>
      <div class="integration-badge">
        <span class="integration-dot"></span> Notion
      </div>
      <div class="integration-badge">
        <span class="integration-dot"></span> n8n
      </div>
// ... (existing code)

      // New: Execute OODA Goal
      async function executeGoal(goalInput) {
    if (!goalInput) goalInput = document.getElementById('goalInput').value;
      if (!goalInput) return;

      addMessage(`🧠 <b>Goal Accepted:</b> "${goalInput}"<br>Thinking & Planning...`, 'system');

        try {
        const resp = await fetch(`${API_BASE}/agent/goal`, {
          method: 'POST',
        headers: {'Content-Type': 'application/json' },
        body: JSON.stringify({goal: goalInput })
        });
        const data = await resp.json();

        if (data.status === 'started') {
          addMessage(`🚀 <b>Execution Started</b><br>Monitor terminal for live verification updates.`, 'system');
        } else {
          addMessage(`❌ Failed: ${data.message}`, 'error');
        }
    } catch (e) {
          addMessage(`❌ Error: ${e}`, 'error');
    }
}

        // Bind Enter key on goalInput if exists
        const goalInput = document.getElementById('goalInput');
        if (goalInput) {
          goalInput.addEventListener('keypress', (e) => {
            if (e.key === 'Enter') executeGoal();
          });
}

        // ... (fetchRoutines)
        const messagesEl = document.getElementById('messages');
        const inputEl = document.getElementById('userInput');
        const sendBtn = document.getElementById('sendBtn');
        const statusBadge = document.getElementById('statusBadge');
        const statusText = document.getElementById('statusText');
        const recsContainer = document.getElementById('recsContainer');

        // Add message to chat
        function addMessage(text, type = 'agent') {
  const msg = document.createElement('div');
        msg.className = `message ${type} `;
        msg.innerHTML = text;
        messagesEl.appendChild(msg);
        messagesEl.scrollTop = messagesEl.scrollHeight;
}

        // Check API health
        async function checkHealth() {
  try {
    const resp = await fetch(`${API_BASE}/health`);
        if (resp.ok) {
          statusBadge.classList.add('connected');
        statusText.textContent = 'Running';
        return true;
  }
} catch (e) {
          statusBadge.classList.remove('connected');
        statusText.textContent = 'Disconnected';
}
        return false;
}

        // Fetch system status
        async function fetchStatus() {
  try {
    const resp = await fetch(`${API_BASE}/status`);
        if (resp.ok) {
      const data = await resp.json();
        document.getElementById('cpuValue').textContent = data.cpu.toFixed(1) + '%';
        document.getElementById('ramValue').textContent = data.ram.toFixed(1) + '%';
        document.getElementById('cpuBar').style.width = Math.min(data.cpu, 100) + '%';
        document.getElementById('ramBar').style.width = Math.min(data.ram, 100) + '%';
    }
  } catch (e) {
          console.error('Failed to fetch status:', e);
  }
}

        // Fetch recommendations
        async function fetchRecommendations() {
  try {
    const resp = await fetch(`${API_BASE}/recommendations`);
        if (resp.ok) {
      const recs = await resp.json();
        if (recs.length === 0) {
          recsContainer.innerHTML = '<div class="empty">추천 없음</div>';
      } else {
          recsContainer.innerHTML = recs.map(rec => `
          <div class="rec-item">
            <div class="rec-title">${rec.title}</div>
            <div class="rec-confidence">${(rec.confidence * 100).toFixed(0)}%</div>
            <div class="rec-actions">
              <button onclick="approveRec(${rec.id})" class="rec-btn approve">✓</button>
              <button onclick="rejectRec(${rec.id})" class="rec-btn reject">✗</button>
            </div>
          </div>
        `).join('');
      }
    }
  } catch (e) {
          recsContainer.innerHTML = '<div class="error">연결 실패</div>';
  }
}

// Approve/reject recommendations
// Approve/reject recommendations
window.approveRec = async (id) => {
  // 1. UI Feedback: Disable button and show loading
  const btn = document.querySelector(`button[onclick="approveRec(${id})"]`);
        const originalText = btn ? btn.innerHTML : '✓';
        if (btn) {
          btn.disabled = true;
        btn.innerHTML = '<span class="spin">⏳</span>'; // Simple loader
        btn.classList.add('loading-btn');
  }

        // 2. Inform User
        addMessage(`⚙️ <b>추천 #${id}</b> 설치를 시작합니다...<br>AI 설계 및 n8n 연동 중 (약 3~5초 소요)`, 'agent');

          try {
    const resp = await fetch(`${API_BASE}/recommendations/${id}/approve`, {method: 'POST' });
          if (resp.ok) {
            addMessage(`✅ <b>추천 #${id}</b> 설치 완료!<br>n8n 워크플로우가 활성화되었습니다 (Inactive Mode).`, 'agent');
          fetchRecommendations();
    } else {
            // Error Handling with JSON
            let errorMsg = `❌ 설치 실패. 다시 시도해주세요.`;
          try {
             const errData = await resp.json();
          if (errData.error) errorMsg = `❌ 설치 실패: ${errData.error}`;
          if (errData.details) errorMsg += `<br><small style="color:#aaa; font-size: 0.8em;">Details: ${errData.details}</small>`;
        } catch(e) {console.warn("Failed to parse error JSON", e); }

            addMessage(errorMsg, 'agent');

            if (btn) {
              btn.disabled = false;
            btn.innerHTML = originalText;
            btn.classList.remove('loading-btn');
        }
    }
  } catch (e) {
              addMessage(`❌ 네트워크 오류 발생: ${e}`, 'agent');
            if (btn) {
              btn.disabled = false;
            btn.innerHTML = originalText;
            btn.classList.remove('loading-btn');
      }
  }

            fetchRecommendations();
};

window.rejectRec = async (id) => {
  // Simple rejection usually instant, but good habit
  const btn = document.querySelector(`button[onclick="rejectRec(${id})"]`);
            if (btn) btn.disabled = true;

            await fetch(`${API_BASE}/recommendations/${id}/reject`, {method: 'POST' });
            addMessage(`🗑️ 추천 #${id} 거절됨`, 'agent');
            fetchRecommendations();
};

            // Send chat message
            async function sendCommand(message) {
              addMessage(message, 'user');

            try {
    const resp = await fetch(`${API_BASE}/chat`, {
              method: 'POST',
            headers: {'Content-Type': 'application/json' },
            body: JSON.stringify({message}),
    });

            if (resp.ok) {
      const data = await resp.json();
            addMessage(data.response, 'agent');

            // Update status after certain commands
            if (data.command === 'system_status') {
              fetchStatus();
      }
    } else {
              addMessage('❌ 서버 오류가 발생했습니다.', 'agent');
    }
  } catch (e) {
              addMessage('❌ 서버에 연결할 수 없습니다. 에이전트가 실행 중인지 확인하세요.', 'agent');
  }
}

            async function runPatternAnalysis() {
              addMessage('🔍 패턴 분석을 시작합니다... (잠시만 기다려주세요)', 'agent');

            try {
    const resp = await fetch(`${API_BASE}/patterns/analyze`, {method: 'POST' });
            if (!resp.ok) {
              addMessage('❌ 패턴 분석에 실패했습니다. 잠시 후 다시 시도해주세요.', 'agent');
            return;
    }

            const data = await resp.json();
    if (Array.isArray(data) && data.length > 0) {
      const lines = data.map(item => `• ${item}`).join('<br>');
              addMessage(`✅ 패턴 분석 완료:<br>${lines}`, 'agent');
    } else {
                  addMessage('✅ 패턴 분석 완료. 새로 감지된 패턴은 없습니다.', 'agent');
    }
  } catch (e) {
                  addMessage(`❌ 네트워크 오류로 패턴 분석에 실패했습니다: ${e}`, 'agent');
  }

                fetchRecommendations();
}

                // System Alerts Check
                async function checkSystemAlerts() {
  try {
    const resp = await fetch(`${API_BASE}/system/health`);
                if (!resp.ok) return;

                const data = await resp.json();
                const alertContainer = document.querySelector('.main-content');
                const existingBanner = document.querySelector('.alert-banner');

    if (data.missing_deps && data.missing_deps.length > 0) {
      // We have missing deps
      const dep = data.missing_deps[0]; // Show first one
                const alertHTML = `
                <div class="alert-content">
                  <span class="alert-icon">⚠️</span>
                  <span><b>${dep.name}</b> 미설치: ${dep.is_critical ? '필수 도구입니다.' : '설치 권장'}</span>
                </div>
                <button class="alert-action-btn" onclick="copyToClipboard('${dep.install_cmd}')">
                  설치 명령 복사
                </button>
                `;

                if (existingBanner) {
                  existingBanner.innerHTML = alertHTML;
      } else {
        const banner = document.createElement('div');
                banner.className = 'alert-banner';
                banner.innerHTML = alertHTML;
                alertContainer.insertBefore(banner, alertContainer.firstChild);
      }
    } else {
      // No missing deps, remove banner if exists
      if (existingBanner) existingBanner.remove();
    }
  } catch (e) {
                  console.error('Failed to check system alerts:', e);
  }
}

window.copyToClipboard = (text) => {
                  navigator.clipboard.writeText(text);
                addMessage(`📋 클립보드에 복사됨: <code>${text}</code><br>터미널에 붙여넣어 실행하세요.`, 'agent');
};

                  setInterval(checkSystemAlerts, 10000); // Check every 10s
                  checkSystemAlerts(); // Initial check

// Event Listeners
sendBtn.addEventListener('click', () => {
  const text = inputEl.value.trim();
                  if (text) {
                    sendCommand(text);
                  inputEl.value = '';
  }
});

inputEl.addEventListener('keypress', (e) => {
  if (e.key === 'Enter') {
    const text = inputEl.value.trim();
                  if (text) {
                    sendCommand(text);
                  inputEl.value = '';
    }
  }
});

// Quick action buttons
document.querySelectorAll('.action-btn').forEach(btn => {
                    btn.addEventListener('click', () => {
                      const cmd = btn.dataset.cmd;
                      if (cmd === 'analyze_patterns') {
                        runPatternAnalysis();
                        return;
                      }
                      sendCommand(cmd);
                    });
});

                  // Initial load
                  checkHealth();
                  fetchStatus();
                  fetchRecommendations();

                  // Periodic updates
                  setInterval(fetchStatus, 5000);
                  setInterval(fetchRecommendations, 10000);
                  setInterval(checkHealth, 30000);
