use crate::nl_automation::{IntentResult, IntentType, SlotMap};
use chrono::{Datelike, Duration, Local, Weekday};
use regex::Regex;

pub fn classify_intent(text: &str) -> IntentResult {
    let lower = text.to_lowercase();
    let mut intent = IntentType::GenericTask;
    let mut confidence = 0.2f32;
    let mut slots = SlotMap::new();

    let route_hint = parse_route(text).is_some();
    if route_hint
        || contains_any(
            &lower,
            &[
                "flight",
                "air",
                "항공",
                "비행기",
                "호텔",
                "숙박",
                "항공권",
                "항공편",
                "편도",
                "왕복",
                "출발",
                "도착",
            ],
        )
    {
        intent = IntentType::FlightSearch;
        confidence = 0.65;
        slots = extract_flight_slots(text);
    } else if contains_any(
        &lower,
        &[
            "shopping",
            "price",
            "최저가",
            "가격",
            "쇼핑",
            "구매",
            "상품",
        ],
    ) || looks_like_product(&lower)
    {
        intent = IntentType::ShoppingCompare;
        confidence = 0.6;
        slots = extract_shopping_slots(text);
    } else if contains_any(
        &lower,
        &["form", "가입", "신청", "작성", "폼", "입력", "예약"],
    ) {
        intent = IntentType::FormFill;
        confidence = 0.55;
        slots = extract_form_slots(text);
    }

    IntentResult {
        intent,
        confidence,
        slots,
    }
}

fn contains_any(text: &str, keywords: &[&str]) -> bool {
    keywords.iter().any(|kw| text.contains(kw))
}

fn looks_like_product(text: &str) -> bool {
    let product_keywords = [
        "아이폰",
        "에어팟",
        "갤럭시",
        "맥북",
        "아이패드",
        "노트북",
        "청소기",
        "프로",
        "울트라",
        "맥스",
        "미니",
        "에디션",
    ];
    if contains_any(text, &product_keywords) {
        return true;
    }
    let model_regex = Regex::new(r"(?i)\b\d+\s*(gb|tb|세대|gen)\b").ok();
    model_regex.and_then(|re| re.find(text)).is_some()
}

fn extract_flight_slots(text: &str) -> SlotMap {
    let mut slots = SlotMap::new();
    let lower = text.to_lowercase();

    if let Some((from, to)) = parse_route(text) {
        slots.insert("from".to_string(), from);
        slots.insert("to".to_string(), to);
    }

    if let Some(date) = parse_date(text) {
        slots.insert("date_start".to_string(), date);
    }

    if let Some(budget) = parse_budget(text) {
        slots.insert("budget_max".to_string(), budget);
    }

    if lower.contains("직항") || lower.contains("nonstop") {
        slots.insert("direct_only".to_string(), "true".to_string());
    }

    if lower.contains("오전") {
        slots.insert("time_window".to_string(), "morning".to_string());
    } else if lower.contains("오후") {
        slots.insert("time_window".to_string(), "afternoon".to_string());
    } else if lower.contains("저녁") || lower.contains("밤") {
        slots.insert("time_window".to_string(), "evening".to_string());
    }

    slots
}

fn extract_shopping_slots(text: &str) -> SlotMap {
    let mut slots = SlotMap::new();
    if let Some(name) = parse_quoted(text) {
        slots.insert("product_name".to_string(), name);
    }

    let cleaned = text
        .replace("최저가", "")
        .replace("가격", "")
        .replace("찾아줘", "")
        .replace("검색", "")
        .replace("사줘", "")
        .replace("구매", "")
        .trim()
        .to_string();
    if !cleaned.is_empty() && !slots.contains_key("product_name") {
        slots.insert("product_name".to_string(), cleaned);
    }

    if let Some((min_price, max_price)) = parse_price_range(text) {
        if let Some(min_val) = min_price {
            slots.insert("price_min".to_string(), min_val);
        }
        if let Some(max_val) = max_price {
            slots.insert("price_max".to_string(), max_val);
        }
    } else if let Some(max_budget) = parse_budget(text) {
        slots.insert("price_max".to_string(), max_budget);
    }
    slots
}

fn extract_form_slots(text: &str) -> SlotMap {
    let mut slots = SlotMap::new();
    if let Some(url) = parse_url(text) {
        slots.insert("target_url".to_string(), url);
    }
    if !text.trim().is_empty() {
        slots.insert("form_purpose".to_string(), text.trim().to_string());
    }
    slots
}

fn parse_route(text: &str) -> Option<(String, String)> {
    let patterns = [
        r"(?i)(?:from\s+)?(?P<from>[A-Za-z가-힣\s]+?)\s*(?:to|->|→|에서|출발|출발지|출발공항)\s*(?P<to>[A-Za-z가-힣\s]+)",
        r"(?i)(?P<from>[A-Za-z가-힣\s]+?)\s*(?:-|~|→)\s*(?P<to>[A-Za-z가-힣\s]+)",
        r"(?i)(?P<from>[A-Za-z가-힣]+)\s*에서\s*(?P<to>[A-Za-z가-힣]+)(?:로|까지)?",
    ];
    for pattern in patterns {
        let route_regex = Regex::new(pattern).ok()?;
        if let Some(caps) = route_regex.captures(text) {
            let raw_from = caps.name("from")?.as_str();
            let raw_to = caps.name("to")?.as_str();
            let from = normalize_place(raw_from);
            let to = normalize_place(raw_to);
            if !from.is_empty() && !to.is_empty() {
                return Some((from, to));
            }
        }
    }
    None
}

fn normalize_place(value: &str) -> String {
    let mut cleaned = value.trim().to_string();
    let cut_tokens = [
        "왕복",
        "편도",
        "항공권",
        "비행기",
        "티켓",
        "검색",
        "찾아줘",
        "예약",
        "가격",
        "최저가",
        "항공",
    ];
    if let Some(idx) = cut_tokens
        .iter()
        .filter_map(|token| cleaned.find(token).map(|i| (i, token.len())))
        .map(|(i, _)| i)
        .min()
    {
        cleaned = cleaned[..idx].to_string();
    }
    let lowered = cleaned.to_lowercase();
    let en_tokens = [
        "round trip",
        "one way",
        "flight",
        "flights",
        "ticket",
        "search",
    ];
    for token in en_tokens {
        if let Some(idx) = lowered.find(token) {
            cleaned = cleaned[..idx].to_string();
            break;
        }
    }
    let suffixes = [
        "에서",
        "출발",
        "출발지",
        "도착",
        "도착지",
        "까지",
        "행",
        "으로",
        "로",
        "가는",
        "오는",
    ];
    let mut trimmed = cleaned.trim().to_string();
    for suffix in suffixes {
        if trimmed.ends_with(suffix) {
            trimmed = trimmed.trim_end_matches(suffix).trim().to_string();
        }
    }
    let mut parts: Vec<&str> = trimmed.split_whitespace().collect();
    while !parts.is_empty() && is_noise_token(parts[0]) {
        parts.remove(0);
    }
    parts.join(" ")
}

fn is_noise_token(token: &str) -> bool {
    matches!(
        token,
        "일" | "월"
            | "년"
            | "오늘"
            | "내일"
            | "모레"
            | "다음주"
            | "이번주"
            | "주말"
            | "다음달"
            | "이번달"
    )
}

fn parse_date(text: &str) -> Option<String> {
    let ymd_regex = Regex::new(r"(\d{4})[./-](\d{1,2})[./-](\d{1,2})").ok()?;
    if let Some(caps) = ymd_regex.captures(text) {
        let year: i32 = caps.get(1)?.as_str().parse().ok()?;
        let month: u32 = caps.get(2)?.as_str().parse().ok()?;
        let day: u32 = caps.get(3)?.as_str().parse().ok()?;
        return Some(format!("{:04}-{:02}-{:02}", year, month, day));
    }

    let kr_regex = Regex::new(r"(\d{1,2})월\s*(\d{1,2})일").ok()?;
    if let Some(caps) = kr_regex.captures(text) {
        let year = chrono::Utc::now().year();
        let month: u32 = caps.get(1)?.as_str().parse().ok()?;
        let day: u32 = caps.get(2)?.as_str().parse().ok()?;
        return Some(format!("{:04}-{:02}-{:02}", year, month, day));
    }

    let md_regex = Regex::new(r"(\d{1,2})/(\d{1,2})").ok()?;
    if let Some(caps) = md_regex.captures(text) {
        let year = chrono::Utc::now().year();
        let month: u32 = caps.get(1)?.as_str().parse().ok()?;
        let day: u32 = caps.get(2)?.as_str().parse().ok()?;
        return Some(format!("{:04}-{:02}-{:02}", year, month, day));
    }

    if let Some(relative) = parse_relative_weekday(text) {
        return Some(relative);
    }

    None
}

fn parse_relative_weekday(text: &str) -> Option<String> {
    let weekday = if text.contains("월요일") {
        Weekday::Mon
    } else if text.contains("화요일") {
        Weekday::Tue
    } else if text.contains("수요일") {
        Weekday::Wed
    } else if text.contains("목요일") {
        Weekday::Thu
    } else if text.contains("금요일") {
        Weekday::Fri
    } else if text.contains("토요일") {
        Weekday::Sat
    } else if text.contains("일요일") {
        Weekday::Sun
    } else {
        return None;
    };

    let today = Local::now().date_naive();
    let today_wd = today.weekday().num_days_from_monday() as i64;
    let target_wd = weekday.num_days_from_monday() as i64;
    let mut delta = target_wd - today_wd;
    if delta <= 0 {
        delta += 7;
    }
    if text.contains("다음주") {
        delta += 7;
    }
    let target = today + Duration::days(delta);
    Some(target.format("%Y-%m-%d").to_string())
}

fn parse_budget(text: &str) -> Option<String> {
    let won_regex = Regex::new(r"(\d{1,3}(?:,\d{3})*|\d+)\s*(만원|원)").ok()?;
    if let Some(caps) = won_regex.captures(text) {
        let raw = caps.get(1)?.as_str().replace(',', "");
        let unit = caps.get(2)?.as_str();
        let amount: i64 = raw.parse().ok()?;
        let total = if unit == "만원" {
            amount * 10_000
        } else {
            amount
        };
        return Some(total.to_string());
    }
    let usd_regex = Regex::new(r"\$(\d+)").ok()?;
    if let Some(caps) = usd_regex.captures(text) {
        return Some(format!("{} USD", caps.get(1)?.as_str()));
    }
    None
}

fn parse_price_range(text: &str) -> Option<(Option<String>, Option<String>)> {
    let lower = text.to_lowercase();
    if let Some(amount) = parse_budget(text) {
        if lower.contains("이하") || lower.contains("최대") {
            return Some((None, Some(amount)));
        }
        if lower.contains("이상") || lower.contains("최소") {
            return Some((Some(amount), None));
        }
    }
    None
}

fn parse_url(text: &str) -> Option<String> {
    let url_regex = Regex::new(r"(https?://[\w\-./?%&=:#~+]+)").ok()?;
    url_regex
        .captures(text)
        .and_then(|caps| caps.get(1).map(|m| m.as_str().to_string()))
}

fn parse_quoted(text: &str) -> Option<String> {
    let quote_regex = Regex::new(r#"["'“”](.+?)["'“”]"#).ok()?;
    if let Some(caps) = quote_regex.captures(text) {
        let value = caps.get(1)?.as_str().trim().to_string();
        if !value.is_empty() {
            return Some(value);
        }
    }
    None
}
