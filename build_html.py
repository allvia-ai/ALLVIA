import re
import json

def parse_markdown(filepath):
    with open(filepath, 'r') as f:
        text = f.read()
    
    # Extract A sections
    a_part = re.search(r'## A\..*?(?=## B\.)', text, re.DOTALL)
    b_part = re.search(r'## B\..*?(?=## C\.)', text, re.DOTALL)
    
    sections = []
    
    if a_part:
        a_text = a_part.group(0)
        # Split A into subsections by ### 1), ### 2), etc.
        sub_sections = re.split(r'### (\d\).*?)\n', a_text)
        for i in range(1, len(sub_sections), 2):
            sec_title = sub_sections[i].strip()
            sec_content = sub_sections[i+1]
            q_matches = re.finditer(r'### (Q\d+(?:-\d+)?)\. (.*?)\n답변: (.*?)(?=\n### |$)', sec_content, re.DOTALL)
            items = []
            for m in q_matches:
                items.append({
                    "id": m.group(1).strip(),
                    "q": m.group(2).strip(),
                    "a": m.group(3).strip().replace('\n', '<br>')
                })
            sections.append({
                "sectionTitle": sec_title,
                "items": items
            })
            
    if b_part:
        b_text = b_part.group(0)
        q_matches = re.finditer(r'### (Q\d+(?:-\d+)?)\. (.*?)\n답변: (.*?)(?=\n### |$)', b_text, re.DOTALL)
        items = []
        for m in q_matches:
            items.append({
                "id": "B-" + m.group(1).strip(),
                "q": m.group(2).strip(),
                "a": m.group(3).strip().replace('\n', '<br>')
            })
        sections.append({
            "sectionTitle": "B. 부트캠프 주관기업 질문",
            "items": items
        })
        
    return sections

# Use the non-bracketed one as it's the updated one, but let's check both
# Parse the main updated master file to get all 80 items.
sections = parse_markdown('/Users/david/Desktop/python/github/Allrounder/Steer/local-os-agent/STEER_OS_EVALUATION_QA_MASTER.md')

# If lengths are weird, we can fallback, but STEER_OS_EVALUATION_QA_MASTER.md has everything + user updates.
# Now define role mapping based on keywords or manually.
rolesData = {
    "tab1": [], # Core Backend (Execution, OS Native, Lock/Concurrency, Hallucination)
    "tab2": [], # Workflow/n8n (Retry, Failure, Pipeline, SPOF, Reconcile)
    "tab3": [], # Collector/Data (Privacy, Zero-Knowledge, Suggestions, Lock-in)
    "tab4": []  # Frontend/Docs/Manager (Business, ROI, UI, Git, Presentation, MVP)
}

for sec in sections:
    for item in sec['items']:
        q = item['q']
        a = item['a']
        text_combined = (q + " " + a).lower()
        
        assigned = False
        
        # Keyword-based heuristics to map to the 4 roles
        # Role 3: Pipeline/Data/Security/Lock-in
        if any(k in text_combined for k in ['마스킹', '보안', 'zero-knowledge', '수집', '콜렉터', '백그라운드', 'lock-in', '록인', '테넌시', '프라이버시', '컴플라이언스', '감사']):
            rolesData["tab3"].append(item['id'])
            assigned = True
        
        # Role 1: Core Backend (OS, 마우스, 환각, 충돌, preflight)
        elif not assigned and any(k in text_combined for k in ['마우스', '키보드', '접근성', 'accessibility', '환각', 'hallucination', '충돌', 'inflight', 'preflight', '디버깅', 'no-key', '물리']):
            rolesData["tab1"].append(item['id'])
            assigned = True
            
        # Role 2: Workflow (n8n, 재시도, 외부 장애, assertion, 복구)
        elif not assigned and any(k in text_combined for k in ['재시도', '백오프', '외부 API', 'n8n', '스레드', 'spof', 'reconcile', '상태 일관성', 'idempotency', 'assertion', '무한 루프']):
            rolesData["tab2"].append(item['id'])
            assigned = True
            
        # Role 4: Business/Frontend/Docs/Management (UI, 기획, 사업성, LTV, ROI, 테스트, 역할, 매출)
        elif not assigned and any(k in text_combined for k in ['매출', '비용', 'ltv', 'ui', '프론트', '문서', 'readme', '데모', '발표', '수익', '시장', '투자', 'icp', 'kpi', '목표', '경쟁사', '팀 역할', '깃헙', '코드리뷰', 'mvp', '테스트']):
            rolesData["tab4"].append(item['id'])
            assigned = True
            
        if not assigned:
            # Default fallback
            if item['id'].startswith('B-'):
                rolesData["tab4"].append(item['id'])
            else:
                rolesData["tab2"].append(item['id'])

html_template = """<!DOCTYPE html>
<html lang="ko">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Allvia (구 STEER OS) 평가 Q&A 대시보드</title>
    <style>
        :root {
            --primary-bg: #f8fafc;
            --container-bg: #ffffff;
            --text-main: #1e293b;
            --text-muted: #64748b;
            --accent-color: #3b82f6;
            --accent-hover: #2563eb;
            --border-color: #e2e8f0;
            --q-bg: #f8fafc;
            --a-bg: #ffffff;
            --highlight: #eff6ff;
            --shadow: 0 4px 6px -1px rgba(0, 0, 0, 0.1), 0 2px 4px -1px rgba(0, 0, 0, 0.06);
            --radius-lg: 12px;
            --radius-md: 8px;
        }

        body {
            font-family: 'Pretendard', -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
            background-color: var(--primary-bg);
            color: var(--text-main);
            line-height: 1.6;
            margin: 0;
            padding: 2rem 1rem;
        }

        .container {
            max-width: 1000px;
            margin: 0 auto;
            background: var(--container-bg);
            border-radius: var(--radius-lg);
            box-shadow: var(--shadow);
            padding: 2.5rem;
        }

        header {
            text-align: center;
            margin-bottom: 2rem;
            padding-bottom: 1.5rem;
            border-bottom: 2px solid var(--border-color);
        }

        h1 {
            color: var(--text-main);
            font-size: 2.25rem;
            font-weight: 800;
            margin-bottom: 0.5rem;
            letter-spacing: -0.025em;
        }

        .meta-info {
            color: var(--text-muted);
            font-size: 0.95rem;
            margin-bottom: 2rem;
        }

        .view-switcher {
            display: flex;
            justify-content: center;
            background: #f1f5f9;
            padding: 0.5rem;
            border-radius: 99px;
            max-width: 400px;
            margin: 0 auto 2.5rem auto;
            position: relative;
        }

        .view-btn {
            flex: 1;
            padding: 0.75rem 1.5rem;
            border: none;
            background: transparent;
            font-weight: 700;
            font-size: 1rem;
            color: var(--text-muted);
            cursor: pointer;
            border-radius: 99px;
            transition: all 0.3s ease;
        }

        .view-btn.active {
            color: var(--text-main);
            background: white;
            box-shadow: 0 2px 5px rgba(0,0,0,0.1);
        }

        .view-container { display: none; animation: fadeIn 0.4s ease-out; }
        .view-container.active { display: block; }
        @keyframes fadeIn { from { opacity: 0; transform: translateY(10px); } to { opacity: 1; transform: translateY(0); } }

        .controls { display: flex; justify-content: flex-end; margin-bottom: 1.5rem; gap: 0.75rem; }
        .btn { background: white; border: 1px solid var(--border-color); padding: 0.5rem 1rem; border-radius: var(--radius-md); cursor: pointer; font-size: 0.9rem; font-weight: 600; color: var(--text-muted); transition: all 0.2s; }
        .btn:hover { color: var(--accent-color); border-color: var(--accent-color); background: var(--highlight); }

        .tabs { display: flex; flex-wrap: wrap; gap: 0.5rem; margin-bottom: 2rem; justify-content: center; }
        .tab-btn { background: white; border: 1px solid var(--border-color); color: var(--text-muted); padding: 0.75rem 1.25rem; border-radius: var(--radius-md); font-size: 0.95rem; font-weight: 700; cursor: pointer; transition: all 0.2s; flex: 1 1 calc(25% - 0.5rem); text-align: center; min-width: 200px; }
        .tab-btn:hover { border-color: #cbd5e1; color: var(--text-main); }
        .tab-btn.active { background: var(--accent-color); color: white; border-color: var(--accent-color); box-shadow: 0 4px 6px rgba(59, 130, 246, 0.3); }

        .role-desc { background: var(--highlight); color: var(--accent-hover); padding: 1.25rem; border-radius: var(--radius-md); margin-bottom: 2rem; font-weight: 600; border-left: 4px solid var(--accent-color); line-height: 1.5; font-size: 0.95rem; }
        .tab-content { display: none; }
        .tab-content.active { display: block; animation: fadeIn 0.3s; }

        .section-title { color: var(--accent-color); font-size: 1.5rem; font-weight: 800; margin: 3rem 0 1.5rem 0; padding-left: 0.75rem; border-left: 4px solid var(--accent-color); }
        .section-title:first-child { margin-top: 1rem; }

        .faq-item { margin-bottom: 1rem; border: 1px solid var(--border-color); border-radius: var(--radius-md); overflow: hidden; transition: all 0.2s ease; background: var(--q-bg); }
        .faq-item:hover { border-color: #cbd5e1; box-shadow: 0 2px 8px rgba(0,0,0,0.04); }
        .faq-question { background-color: transparent; padding: 1.25rem 1.5rem; width: 100%; text-align: left; border: none; cursor: pointer; font-size: 1.05rem; font-weight: 700; color: var(--text-main); display: flex; justify-content: space-between; align-items: center; transition: all 0.2s ease; }
        .faq-question.active { background-color: white; color: var(--accent-color); border-bottom: 1px solid var(--border-color); }
        .icon { font-size: 1.5rem; font-weight: 300; transition: transform 0.3s ease; color: var(--text-muted); margin-left: 1rem; }
        .faq-question.active .icon { transform: rotate(45deg); color: var(--accent-color); }
        .faq-answer { max-height: 0; overflow: hidden; transition: max-height 0.35s cubic-bezier(0.4, 0, 0.2, 1); background-color: var(--a-bg); }
        .answer-content { padding: 1.5rem; font-size: 1.05rem; color: #334155; line-height: 1.7; }
        .answer-content strong { color: var(--text-main); background-color: #f1f5f9; padding: 0.1rem 0.4rem; border-radius: 4px; font-weight: 700; }
        .badge { display: inline-block; background: #e2e8f0; color: #475569; font-size: 0.75rem; font-weight: 800; padding: 0.2rem 0.6rem; border-radius: 9999px; margin-right: 0.75rem; vertical-align: text-bottom; letter-spacing: 0.5px; }
        .faq-question.active .badge { background: var(--accent-color); color: white; }

        @media (max-width: 768px) { .container { padding: 1.5rem; } .tab-btn { flex: 1 1 100%; } .view-switcher { flex-direction: column; border-radius: 12px; } .view-btn { border-radius: 8px; } }
    </style>
</head>
<body>

<div class="container">
    <header>
        <h1>Allvia (구 STEER OS) 방어 Q&amp;A</h1>
        <div class="meta-info">평가장에서 사용할 보기 모드를 선택하세요. (총 80개 전체 문항 복구 완료)</div>
    </header>

    <div class="view-switcher">
        <button class="view-btn active" onclick="switchMainView('view-list')">📖 전체 문항 순차 보기</button>
        <button class="view-btn" onclick="switchMainView('view-role')">👥 담당 롤(Role)별 보기</button>
    </div>

    <div id="view-list" class="view-container active">
        <div class="controls">
            <button class="btn" onclick="toggleAll('list', true)">전체 열기</button>
            <button class="btn" onclick="toggleAll('list', false)">전체 닫기</button>
        </div>
        <div id="list-content-area"></div>
    </div>

    <div id="view-role" class="view-container">
        <div class="tabs">
            <button class="tab-btn active" onclick="switchRoleTab('tab1')">1. 자연어 실행 (Core)<br><span style="font-size:0.8rem; font-weight:400;">(마우스/키보드 실전제어)</span></button>
            <button class="tab-btn" onclick="switchRoleTab('tab2')">2. 워크플로우 (n8n)<br><span style="font-size:0.8rem; font-weight:400;">(패턴 실현 및 에러복구)</span></button>
            <button class="tab-btn" onclick="switchRoleTab('tab3')">3. 수집/파이프라인<br><span style="font-size:0.8rem; font-weight:400;">(로그수집 및 보안)</span></button>
            <button class="tab-btn" onclick="switchRoleTab('tab4')">4. 프론트/총괄<br><span style="font-size:0.8rem; font-weight:400;">(문서, UI, 기획지표)</span></button>
        </div>

        <div class="controls">
            <button class="btn" onclick="toggleAll('role', true)">현재 탭 모두 열기</button>
            <button class="btn" onclick="toggleAll('role', false)">현재 탭 모두 닫기</button>
        </div>

        <div id="tab1" class="tab-content active">
            <div class="role-desc">📌 [담당자 1 - 코어(자연어 요청 구동)]<br>사용자의 자연어를 판별해 플랜을 구성하고 OS 네이티브(마우스/키보드) 물리 이동을 제어합니다. 인프라 충돌 통제와 AI 환각(대사고 방지) 등을 방어합니다.</div>
            <div id="role-content1"></div>
        </div>
        <div id="tab2" class="tab-content">
            <div class="role-desc">📌 [담당자 2 - 워크플로우(n8n 연동)]<br>플랜이 컴파일 된 워크플로우를 중단 없이 체인으로 엮어 돌리는 파트. Retry(재시도), Backoff, 외부 파이프라인/API 장애 격리와 멱등성을 방어합니다.</div>
            <div id="role-content2"></div>
        </div>
        <div id="tab3" class="tab-content">
            <div class="role-desc">📌 [담당자 3 - 콜렉터(수집 패턴 및 선제안)]<br>유저 사용 패턴 집계 및 '자동화 선제안' 기능 어필. Enterprise 진입의 핵심인 Zero-Knowledge (마스킹)와 데이터베이스 프라이버시, 테넌트 격리를 철통 방어합니다.</div>
            <div id="role-content3"></div>
        </div>
        <div id="tab4" class="tab-content">
            <div class="role-desc">📌 [담당자 4 - 프론트, 웹 UI, 문서 총괄]<br>Tauri 기반 프론트 최적화 로직, Github 리뷰와 문서화 파이프라인(Docs-as-code), 그리고 최종 LTV/CAC 및 유료화 등 사업 기획 지표를 방어합니다.</div>
            <div id="role-content4"></div>
        </div>
    </div>
</div>

<script>
    const listData = __LIST_DATA__;
    const rolesData = __ROLES_DATA__;

    const allQuestionsMap = {};
    listData.forEach(section => {
        section.items.forEach(item => {
            allQuestionsMap[item.id] = item;
        });
    });

    function createFAQElement(item) {
        return `
            <div class="faq-item">
                <button class="faq-question" onclick="toggleAnswer(this)">
                    <span><span class="badge">${item.id}</span> ${item.q}</span>
                    <span class="icon">+</span>
                </button>
                <div class="faq-answer">
                    <div class="answer-content">
                        <p>${item.a}</p>
                    </div>
                </div>
            </div>
        `;
    }

    function renderListView() {
        const container = document.getElementById('list-content-area');
        let html = '';
        listData.forEach(sec => {
            html += `<h2 class="section-title">${sec.sectionTitle}</h2>`;
            sec.items.forEach(item => { html += createFAQElement(item); });
        });
        container.innerHTML = html;
    }

    function renderRoleView() {
        ['tab1', 'tab2', 'tab3', 'tab4'].forEach(tabId => {
            const container = document.getElementById(tabId.replace('tab', 'role-content'));
            let html = '';
            rolesData[tabId].forEach(qId => {
                if (allQuestionsMap[qId]) {
                    html += createFAQElement(allQuestionsMap[qId]);
                }
            });
            container.innerHTML = html;
        });
    }

    function switchMainView(viewId) {
        document.querySelectorAll('.view-btn').forEach(b => b.classList.remove('active'));
        document.querySelector(`.view-btn[onclick="switchMainView('${viewId}')"]`).classList.add('active');

        document.querySelectorAll('.view-container').forEach(c => c.classList.remove('active'));
        document.getElementById(viewId).classList.add('active');
    }

    function switchRoleTab(tabId) {
        document.querySelectorAll('.tab-btn').forEach(b => b.classList.remove('active'));
        document.querySelectorAll('.tab-content').forEach(c => c.classList.remove('active'));
        document.querySelector(`.tab-btn[onclick="switchRoleTab('${tabId}')"]`).classList.add('active');
        document.getElementById(tabId).classList.add('active');
    }

    function toggleAnswer(button) {
        button.classList.toggle('active');
        const answer = button.nextElementSibling;
        const icon = button.querySelector('.icon');
        
        if (button.classList.contains('active')) {
            answer.style.maxHeight = answer.scrollHeight + 30 + "px";
            icon.textContent = "×";
        } else {
            answer.style.maxHeight = null;
            icon.textContent = "+";
        }
    }

    function toggleAll(context, open) {
        let buttons;
        if (context === 'list') {
            buttons = document.getElementById('view-list').querySelectorAll('.faq-question');
        } else {
            const activeTab = document.querySelector('.tab-content.active');
            buttons = activeTab.querySelectorAll('.faq-question');
        }
        
        buttons.forEach(button => {
            const answer = button.nextElementSibling;
            const icon = button.querySelector('.icon');
            
            if (open && !button.classList.contains('active')) {
                button.classList.add('active');
                answer.style.maxHeight = answer.scrollHeight + 30 + "px";
                icon.textContent = "×";
            } else if (!open && button.classList.contains('active')) {
                button.classList.remove('active');
                answer.style.maxHeight = null;
                icon.textContent = "+";
            }
        });
    }

    document.addEventListener("DOMContentLoaded", () => {
        renderListView();
        renderRoleView();
    });
</script>
</body>
</html>"""

html_out = html_template.replace('__LIST_DATA__', json.dumps(sections, ensure_ascii=False)).replace('__ROLES_DATA__', json.dumps(rolesData, ensure_ascii=False))

with open('/Users/david/.gemini/antigravity/brain/146b649e-b4c9-4248-8058-a5e194a0143f/Allvia_Evaluation_QA.html', 'w') as f:
    f.write(html_out)

print(f"Done. Processed {sum(len(s['items']) for s in sections)} questions.")
