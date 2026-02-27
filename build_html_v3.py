import re
import json

def parse_main_qa(filepath):
    with open(filepath, 'r', encoding='utf-8', errors='ignore') as f:
        text = f.read()
    
    a_part = re.search(r'## A\..*?(?=## B\.)', text, re.DOTALL)
    b_part = re.search(r'## B\..*?(?=## C\.)', text, re.DOTALL)
    
    sections = []
    
    if a_part:
        a_text = a_part.group(0)
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

def parse_adr(filepath):
    with open(filepath, 'r', encoding='utf-8', errors='ignore') as f:
        text = f.read()
        
    parts = re.split(r'\n## ADR-', text)
    adrs = []
    for p in parts[1:]:
        lines = p.split('\n')
        title_line = lines[0].strip()
        adr_id = "ADR-" + title_line.split('.')[0]
        q_text = title_line
        
        content = '\n'.join(lines[1:])
        content_html = content.replace("### ", "<br><strong style='color:#3b82f6;'>■ ").replace("\n", "<br>")
        content_html = content_html.replace("<br><br><strong", "<br><strong")
        content_html = re.sub(r'-(.*?)<br>', r'<li>\1</li>', content_html)
        content_html = content_html.replace("</strong>", "</strong><br>")
        
        adrs.append({
            "id": adr_id,
            "q": q_text,
            "a": content_html
        })
    return adrs

def parse_overview(filepath):
    with open(filepath, 'r', encoding='utf-8', errors='ignore') as f:
        text = f.read()
    
    # Extract sections 13.1 to 13.5
    overview_items = []
    
    sections_to_extract = [
        ("13.1", "C4-Style Summary (컨테이너/컴포넌트 구조)"),
        ("13.2", "Runtime Sequence (런타임 시퀀스 흐름)"),
        ("13.3", "State Model (상태 모델 및 라이프사이클)"),
        ("13.4", "Reliability Architecture (신뢰성 아키텍처)"),
        ("13.5", "Security Architecture (보안 아키텍처)"),
        ("13.7", "Evolution Roadmap (향후 진화 로드맵)")
    ]
    
    for sec_num, sec_title in sections_to_extract:
        # Regex to find the section content until the next section starting with "## " or "### "
        pattern = r'## ' + re.escape(sec_num) + r'.*?\n(.*?)((?=\n## 13\.)|(?=\n## 14\.)|$)'
        match = re.search(pattern, text, re.DOTALL)
        if match:
            content = match.group(1).strip()
            content_html = content.replace("### ", "<br><strong style='color:#10b981;'>■ ").replace("\n", "<br>")
            content_html = content_html.replace("<br><br><strong", "<br><strong")
            content_html = re.sub(r'- (.*?)<br>', r'<li>\1</li>', content_html)
            content_html = content_html.replace("</strong>", "</strong><br>")
            
            overview_items.append({
                "id": f"SYS-{sec_num.split('.')[1]}",
                "q": sec_title,
                "a": content_html
            })
            
    return overview_items

main_qa = parse_main_qa('/Users/david/Desktop/python/github/Allrounder/Steer/local-os-agent/STEER_OS_EVALUATION_QA_MASTER.md')
adrs = parse_adr('/Users/david/Desktop/python/github/Allrounder/Steer/local-os-agent/STEER_OS_TECH_ARCH_DECISION_PLAYBOOK.md')
overviews = parse_overview('/Users/david/Desktop/python/github/Allrounder/Steer/local-os-agent/STEER_OS_TECH_STACK_DECISION_STUDY_V2.md')

rolesData = {
    "tab1": [],
    "tab2": [],
    "tab3": [],
    "tab4": []
}

for sec in main_qa:
    for item in sec['items']:
        q = item['q']
        a = item['a']
        text_combined = (q + " " + a).lower()
        assigned = False
        
        if any(k in text_combined for k in ['마스킹', '보안', 'zero-knowledge', '수집', '콜렉터', '백그라운드', 'lock-in', '록인', '테넌시', '프라이버시', '컴플라이언스', '감사']):
            rolesData["tab3"].append(item['id'])
            assigned = True
        elif not assigned and any(k in text_combined for k in ['마우스', '키보드', '접근성', 'accessibility', '환각', 'hallucination', '충돌', 'inflight', 'preflight', '디버깅', 'no-key', '물리']):
            rolesData["tab1"].append(item['id'])
            assigned = True
        elif not assigned and any(k in text_combined for k in ['재시도', '백오프', '외부 api', 'n8n', '스레드', 'spof', 'reconcile', '상태 일관성', 'idempotency', 'assertion', '무한 루프']):
            rolesData["tab2"].append(item['id'])
            assigned = True
        elif not assigned and any(k in text_combined for k in ['매출', '비용', 'ltv', 'ui', '프론트', '문서', 'readme', '데모', '발표', '수익', '시장', '투자', 'icp', 'kpi', '목표', '경쟁사', '팀 역할', '깃헙', '코드리뷰', 'mvp', '테스트']):
            rolesData["tab4"].append(item['id'])
            assigned = True
            
        if not assigned:
            if item['id'].startswith('B-'):
                rolesData["tab4"].append(item['id'])
            else:
                rolesData["tab2"].append(item['id'])

techTabsData = {
    "tech1": ["ADR-01", "ADR-02", "ADR-03", "ADR-04"],
    "tech2": ["ADR-05", "ADR-06", "ADR-07", "ADR-08"],
    "tech3": ["ADR-09", "ADR-10"],
    "tech4": ["ADR-11", "ADR-12", "ADR-13", "ADR-14", "ADR-15"],
    "tech5": [item["id"] for item in overviews] # Add the new overview tab IDs
}

# combine all tech data for the map
all_tech_data = adrs + overviews

html_template = """<!DOCTYPE html>
<html lang="ko">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Allvia (구 STEER OS) 방어 대시보드 (Q&A + Tech Arch)</title>
    <style>
        :root {
            --primary-bg: #f8fafc;
            --container-bg: #ffffff;
            --text-main: #1e293b;
            --text-muted: #64748b;
            --accent-color: #3b82f6;
            --accent-hover: #2563eb;
            --overview-color: #10b981;
            --border-color: #e2e8f0;
            --q-bg: #f8fafc;
            --a-bg: #ffffff;
            --highlight: #eff6ff;
            --shadow: 0 4px 6px -1px rgba(0, 0, 0, 0.1), 0 2px 4px -1px rgba(0, 0, 0, 0.06);
            --radius-lg: 12px;
            --radius-md: 8px;
        }

        body { font-family: 'Pretendard', -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif; background-color: var(--primary-bg); color: var(--text-main); line-height: 1.6; margin: 0; padding: 2rem 1rem; }
        .container { max-width: 1100px; margin: 0 auto; background: var(--container-bg); border-radius: var(--radius-lg); box-shadow: var(--shadow); padding: 2.5rem; }
        header { text-align: center; margin-bottom: 2rem; padding-bottom: 1.5rem; border-bottom: 2px solid var(--border-color); }
        h1 { color: var(--text-main); font-size: 2.25rem; font-weight: 800; margin-bottom: 0.5rem; letter-spacing: -0.025em; }
        .meta-info { color: var(--text-muted); font-size: 0.95rem; margin-bottom: 2rem; }
        
        .view-switcher { display: flex; justify-content: center; background: #f1f5f9; padding: 0.5rem; border-radius: 99px; max-width: 700px; margin: 0 auto 2.5rem auto; gap: 0.25rem; }
        .view-btn { flex: 1; padding: 0.75rem 1rem; border: none; background: transparent; font-weight: 700; font-size: 0.95rem; color: var(--text-muted); cursor: pointer; border-radius: 99px; transition: all 0.3s ease; text-align: center; }
        .view-btn.active { color: var(--text-main); background: white; box-shadow: 0 2px 5px rgba(0,0,0,0.1); }
        
        .view-container { display: none; animation: fadeIn 0.4s ease-out; }
        .view-container.active { display: block; }
        @keyframes fadeIn { from { opacity: 0; transform: translateY(10px); } to { opacity: 1; transform: translateY(0); } }

        .controls { display: flex; justify-content: flex-end; margin-bottom: 1.5rem; gap: 0.75rem; }
        .btn { background: white; border: 1px solid var(--border-color); padding: 0.5rem 1rem; border-radius: var(--radius-md); cursor: pointer; font-size: 0.9rem; font-weight: 600; color: var(--text-muted); transition: all 0.2s; }
        .btn:hover { color: var(--accent-color); border-color: var(--accent-color); background: var(--highlight); }

        .tabs { display: flex; flex-wrap: wrap; gap: 0.5rem; margin-bottom: 2rem; justify-content: center; }
        .tab-btn { background: white; border: 1px solid var(--border-color); color: var(--text-muted); padding: 0.75rem 1rem; border-radius: var(--radius-md); font-size: 0.9rem; font-weight: 700; cursor: pointer; transition: all 0.2s; flex: 1 1 calc(20% - 0.5rem); text-align: center; min-width: 150px; }
        .tab-btn:hover { border-color: #cbd5e1; color: var(--text-main); }
        .tab-btn.active { background: var(--accent-color); color: white; border-color: var(--accent-color); box-shadow: 0 4px 6px rgba(59, 130, 246, 0.3); }
        .tab-btn.overview.active { background: var(--overview-color); border-color: var(--overview-color); box-shadow: 0 4px 6px rgba(16, 185, 129, 0.3); }

        .role-desc { background: var(--highlight); color: var(--accent-hover); padding: 1.25rem; border-radius: var(--radius-md); margin-bottom: 2rem; font-weight: 600; border-left: 4px solid var(--accent-color); line-height: 1.5; font-size: 0.95rem; }
        .role-desc.overview { background: #ecfdf5; color: #047857; border-left-color: var(--overview-color); }
        .tab-content { display: none; }
        .tab-content.active { display: block; animation: fadeIn 0.3s; }

        .section-title { color: var(--accent-color); font-size: 1.5rem; font-weight: 800; margin: 3rem 0 1.5rem 0; padding-left: 0.75rem; border-left: 4px solid var(--accent-color); }
        .section-title:first-child { margin-top: 1rem; }

        .faq-item { margin-bottom: 1rem; border: 1px solid var(--border-color); border-radius: var(--radius-md); overflow: hidden; transition: all 0.2s ease; background: var(--q-bg); }
        .faq-item:hover { border-color: #cbd5e1; box-shadow: 0 2px 8px rgba(0,0,0,0.04); }
        .faq-question { background-color: transparent; padding: 1.25rem 1.5rem; width: 100%; text-align: left; border: none; cursor: pointer; font-size: 1.05rem; font-weight: 700; color: var(--text-main); display: flex; justify-content: space-between; align-items: center; transition: all 0.2s ease; }
        .faq-question.active { background-color: white; color: var(--accent-color); border-bottom: 1px solid var(--border-color); }
        .overview-item .faq-question.active { color: var(--overview-color); }
        .icon { font-size: 1.5rem; font-weight: 300; transition: transform 0.3s ease; color: var(--text-muted); margin-left: 1rem; }
        .faq-question.active .icon { transform: rotate(45deg); color: var(--accent-color); }
        .overview-item .faq-question.active .icon { color: var(--overview-color); }
        .faq-answer { max-height: 0; overflow: hidden; transition: max-height 0.35s cubic-bezier(0.4, 0, 0.2, 1); background-color: var(--a-bg); }
        .answer-content { padding: 1.5rem; font-size: 1.05rem; color: #334155; line-height: 1.7; }
        .answer-content strong { color: var(--text-main); font-weight: 800; }
        .answer-content li { margin-bottom: 0.25rem; }
        .badge { display: inline-block; background: #e2e8f0; color: #475569; font-size: 0.75rem; font-weight: 800; padding: 0.2rem 0.6rem; border-radius: 9999px; margin-right: 0.75rem; vertical-align: text-bottom; letter-spacing: 0.5px; }
        .badge.adr { background: #fee2e2; color: #991b1b; }
        .badge.sys { background: #d1fae5; color: #065f46; }
        .faq-question.active .badge { background: var(--accent-color); color: white; }
        .faq-question.active .badge.adr { background: #ef4444; color: white; }
        .faq-question.active .badge.sys { background: var(--overview-color); color: white; }

        @media (max-width: 768px) { .container { padding: 1.5rem; } .tab-btn { flex: 1 1 100%; } .view-switcher { flex-direction: column; border-radius: 12px; } .view-btn { border-radius: 8px; } }
    </style>
</head>
<body>

<div class="container">
    <header>
        <h1>Allvia (구 STEER OS) 통합 방어 대시보드</h1>
        <div class="meta-info">Q&A 80선 및 Tech Arch Playbook (ADR) + 시스템 오버뷰 완전 통합본</div>
    </header>

    <div class="view-switcher">
        <button class="view-btn active" onclick="switchMainView('view-list')">📖 Q&amp;A 전체 순차 보기</button>
        <button class="view-btn" onclick="switchMainView('view-role')">👥 Q&amp;A 인원 롤(Role)별</button>
        <button class="view-btn" onclick="switchMainView('view-tech')">🛠️ 아키텍처 결정을 '왜' 했는가 (ADR)</button>
    </div>

    <!-- 1. List View -->
    <div id="view-list" class="view-container active">
        <div class="controls">
            <button class="btn" onclick="toggleAll('list', true)">전체 열기</button>
            <button class="btn" onclick="toggleAll('list', false)">전체 닫기</button>
        </div>
        <div id="list-content-area"></div>
    </div>

    <!-- 2. Role View -->
    <div id="view-role" class="view-container">
        <div class="tabs">
            <button class="tab-btn active" onclick="switchRoleTab('role1', 'role')">1. 자연어 실행 (Core)<br><span style="font-size:0.8rem; font-weight:400;">(마우스/키보드 제어)</span></button>
            <button class="tab-btn" onclick="switchRoleTab('role2', 'role')">2. 워크플로우 (n8n)<br><span style="font-size:0.8rem; font-weight:400;">(패턴 실현 및 장애방어)</span></button>
            <button class="tab-btn" onclick="switchRoleTab('role3', 'role')">3. 수집/파이프라인<br><span style="font-size:0.8rem; font-weight:400;">(보안/영지식 마스킹)</span></button>
            <button class="tab-btn" onclick="switchRoleTab('role4', 'role')">4. 프론트/총괄<br><span style="font-size:0.8rem; font-weight:400;">(UI, NRR/CAC 기획)</span></button>
        </div>

        <div class="controls">
            <button class="btn" onclick="toggleAll('role', true)">현재 탭 모두 열기</button>
            <button class="btn" onclick="toggleAll('role', false)">현재 탭 모두 닫기</button>
        </div>

        <div id="role1" class="tab-content active">
            <div class="role-desc">📌 [담당자 1 - 코어(자연어 요청 구동)]<br>플랜을 구성하고 OS 네이티브 물리 이동을 제어합니다. 인프라 충돌 통제와 AI 환각 방어.</div>
            <div id="role-content1"></div>
        </div>
        <div id="role2" class="tab-content">
            <div class="role-desc">📌 [담당자 2 - 워크플로우(n8n 연동)]<br>플랜 컴파일 워크플로우를 중단 없이 체인으로 엮는 파트. 장애 격리와 멱등성을 방어합니다.</div>
            <div id="role-content2"></div>
        </div>
        <div id="role3" class="tab-content">
            <div class="role-desc">📌 [담당자 3 - 콜렉터(보안 및 선제안)]<br>유저 패턴 집계 및 자동화 선제안 기능 어필. Zero-Knowledge 마스킹 데이터 방어 파트.</div>
            <div id="role-content3"></div>
        </div>
        <div id="role4" class="tab-content">
            <div class="role-desc">📌 [담당자 4 - 프론트, GTM, 총괄]<br>메모리 85% 감축을 입증하는 Tauri 프론트엔드 최적화 및 125% NRR 등 비즈니스 지표.</div>
            <div id="role-content4"></div>
        </div>
    </div>

    <!-- 3. Tech ADR View -->
    <div id="view-tech" class="view-container">
        <div class="tabs">
            <button class="tab-btn overview" onclick="switchRoleTab('tech5', 'tech')">🗺️ 시스템 오버뷰<br><span style="font-size:0.8rem; font-weight:400;">(구조, 시퀀스, 로드맵)</span></button>
            <button class="tab-btn active" onclick="switchRoleTab('tech1', 'tech')">⚙️ 백엔드 / 코어<br><span style="font-size:0.8rem; font-weight:400;">(Rust, Axum, DB)</span></button>
            <button class="tab-btn" onclick="switchRoleTab('tech2', 'tech')">🖥️ 데스크톱 / 프론트<br><span style="font-size:0.8rem; font-weight:400;">(Tauri, React, 네이티브)</span></button>
            <button class="tab-btn" onclick="switchRoleTab('tech3', 'tech')">🧠 AI / 워크플로우<br><span style="font-size:0.8rem; font-weight:400;">(n8n, LLM 제어)</span></button>
            <button class="tab-btn" onclick="switchRoleTab('tech4', 'tech')">🛡️ 운영 / 보안 / 신뢰성<br><span style="font-size:0.8rem; font-weight:400;">(멱등성, Gate, 테스트)</span></button>
        </div>

        <div class="controls">
            <button class="btn" onclick="toggleAll('tech', true)">현재 탭 모두 열기</button>
            <button class="btn" onclick="toggleAll('tech', false)">현재 탭 모두 닫기</button>
        </div>

        <div id="tech5" class="tab-content">
            <div class="role-desc overview">🗺️ [시스템 아키텍처 & 실행 시퀀스 오버뷰]<br>평가위원이 "에이전트 전체 구조와 동작 흐름을 설명해달라"고 요청할 때 방어하는 핵심 시스템 조감도와 로드맵입니다.</div>
            <div id="tech-content5"></div>
        </div>
        <div id="tech1" class="tab-content active">
            <div class="role-desc">🔧 [백엔드 / 코어 스택 의사결정]<br>로컬 에이전트의 심장인 Rust 언어 선택부터, 비동기 런타임 Tokio, API 프레임워크 Axum, 퍼시스턴스 SQLite의 채택 이유와 대안 대비 강점을 방어합니다.</div>
            <div id="tech-content1"></div>
        </div>
        <div id="tech2" class="tab-content">
            <div class="role-desc">🖥️ [데스크톱 / 프론트 스택 의사결정]<br>대기 메모리를 극단적으로 낮춘 Tauri의 선택 근거, React 기반 UI 구성 및 macOS 접근성(Accessibility) 네이티브 제어 하이브리드 결정을 방어합니다.</div>
            <div id="tech-content2"></div>
        </div>
        <div id="tech3" class="tab-content">
            <div class="role-desc">🧠 [AI / 워크플로우 스택 의사결정]<br>오케스트레이션 엔진으로 n8n을 채택한 이유와, OpenAI 중심의 LLM 활용 모델(검증 Fallback 포함)의 현실적 트레이드오프를 방어합니다.</div>
            <div id="tech-content3"></div>
        </div>
        <div id="tech4" class="tab-content">
            <div class="role-desc">🛡️ [운영 / 보안 / 신뢰성 아키텍처 결정]<br>단순 실행보다 중요한 '안전한 차단/승인(Approval State)', 상태 정합성 유지를 위한 멱등성(Idempotency), Fail-Closed 설계 기조를 방어합니다.</div>
            <div id="tech-content4"></div>
        </div>
    </div>
</div>

<script>
    const listData = __LIST_DATA__;
    const rolesData = __ROLES_DATA__;
    const allTechDataRaw = __ADR_DATA__;
    const techTabsData = __TECH_TABS_DATA__;

    const allQuestionsMap = {};
    listData.forEach(section => {
        section.items.forEach(item => {
            allQuestionsMap[item.id] = item;
        });
    });

    const allTechMap = {};
    allTechDataRaw.forEach(item => {
        allTechMap[item.id] = item;
    });

    function createFAQElement(item, techType = null) {
        let badgeClass = "badge";
        let wrapperClass = "faq-item";
        
        if (techType === 'adr') {
            badgeClass = "badge adr";
        } else if (techType === 'sys') {
            badgeClass = "badge sys";
            wrapperClass = "faq-item overview-item";
        }
        
        return `
            <div class="${wrapperClass}">
                <button class="faq-question" onclick="toggleAnswer(this)">
                    <span><span class="${badgeClass}">${item.id}</span> ${item.q}</span>
                    <span class="icon">+</span>
                </button>
                <div class="faq-answer">
                    <div class="answer-content">
                        ${techType ? item.a : '<p>' + item.a + '</p>'}
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
        ['tab1', 'tab2', 'tab3', 'tab4'].forEach((tabId, idx) => {
            const container = document.getElementById('role-content' + (idx + 1));
            let html = '';
            rolesData[tabId].forEach(qId => {
                if (allQuestionsMap[qId]) {
                    html += createFAQElement(allQuestionsMap[qId]);
                }
            });
            container.innerHTML = html;
        });
    }

    function renderTechView() {
        ['tech1', 'tech2', 'tech3', 'tech4', 'tech5'].forEach((tabId, idx) => {
            const container = document.getElementById('tech-content' + (idx + 1));
            let html = '';
            techTabsData[tabId].forEach(itemId => {
                if (allTechMap[itemId]) {
                    const tType = itemId.startsWith('SYS-') ? 'sys' : 'adr';
                    html += createFAQElement(allTechMap[itemId], tType);
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
        
        // Show first tab in Tech view by default when switching
        if (viewId === 'view-tech' && !document.querySelector('#view-tech .tab-content.active')) {
            switchRoleTab('tech5', 'tech'); // Default to overview
        }
    }

    function switchRoleTab(tabId, context) {
        const viewContainer = document.getElementById('view-' + context);
        viewContainer.querySelectorAll('.tab-btn').forEach(b => b.classList.remove('active'));
        viewContainer.querySelectorAll('.tab-content').forEach(c => c.classList.remove('active'));
        viewContainer.querySelector(`.tab-btn[onclick="switchRoleTab('${tabId}', '${context}')"]`).classList.add('active');
        document.getElementById(tabId).classList.add('active');
    }

    function toggleAnswer(button) {
        button.classList.toggle('active');
        const answer = button.nextElementSibling;
        const icon = button.querySelector('.icon');
        
        if (button.classList.contains('active')) {
            answer.style.maxHeight = answer.scrollHeight + 50 + "px";
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
            const activeTab = document.getElementById('view-' + context).querySelector('.tab-content.active');
            buttons = activeTab.querySelectorAll('.faq-question');
        }
        
        buttons.forEach(button => {
            const answer = button.nextElementSibling;
            const icon = button.querySelector('.icon');
            
            if (open && !button.classList.contains('active')) {
                button.classList.add('active');
                answer.style.maxHeight = answer.scrollHeight + 50 + "px";
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
        renderTechView();
        
        // Make sure overview tab is active by default in Tech view
        switchRoleTab('tech5', 'tech');
    });
</script>
</body>
</html>"""

html_out = html_template.replace('__LIST_DATA__', json.dumps(main_qa, ensure_ascii=False)) \
                        .replace('__ROLES_DATA__', json.dumps(rolesData, ensure_ascii=False)) \
                        .replace('__ADR_DATA__', json.dumps(all_tech_data, ensure_ascii=False)) \
                        .replace('__TECH_TABS_DATA__', json.dumps(techTabsData, ensure_ascii=False))

with open('/Users/david/.gemini/antigravity/brain/146b649e-b4c9-4248-8058-a5e194a0143f/Allvia_Evaluation_QA.html', 'w') as f:
    f.write(html_out)

print("Generated HTML with System Overview Tab!")
