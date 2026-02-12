#!/bin/bash

# Scenario semantic contracts for run_complex_scenarios.sh
# Required environment variables (provided by runner):
# SUBJECT_S1..SUBJECT_S5, MARKER_S1..MARKER_S5

complex_scenario_expected_tokens() {
    local scenario_num="$1"
    case "$scenario_num" in
        1)
            printf '%s\n' "$SUBJECT_S1" "Calendar opened" "Notes draft ready" "Mail prep pending" "Shared via TextEdit" "$MARKER_S1"
            ;;
        2)
            printf '%s\n' "$SUBJECT_S2" "1. invoice.pdf" "2. screenshot.png" "3. notes.txt" "$MARKER_S2"
            ;;
        3)
            printf '%s\n' "$SUBJECT_S3" "120*1300=" "Done" "$MARKER_S3"
            ;;
        4)
            printf '%s\n' "$SUBJECT_S4" "focus music" "pomodoro timer" "daily review template" "$MARKER_S4"
            ;;
        5)
            printf '%s\n' "$SUBJECT_S5" "Base: 120 USD" "$MARKER_S5"
            ;;
    esac
}

complex_scenario_mail_subject() {
    local scenario_num="$1"
    case "$scenario_num" in
        1) printf '%s\n' "$SUBJECT_S1" ;;
        2) printf '%s\n' "$SUBJECT_S2" ;;
        3) printf '%s\n' "$SUBJECT_S3" ;;
        4) printf '%s\n' "$SUBJECT_S4" ;;
        5) printf '%s\n' "$SUBJECT_S5" ;;
        *) printf '%s\n' "" ;;
    esac
}

complex_scenario_required_artifacts() {
    local scenario_num="$1"
    case "$scenario_num" in
        1|2|3|4|5)
            printf '%s\n' "semantic_tokens" "mail_send" "node_capture"
            ;;
    esac
}
