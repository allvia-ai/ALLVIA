#!/bin/bash

extract_expected_tokens_heuristic() {
    local source_text="$1"
    {
        printf '%s\n' "$source_text" | perl -ne '
            while (/"([^"]+)"|'\''([^'\'']+)'\''/g) {
                my $s = defined($1) && $1 ne "" ? $1 : $2;
                $s =~ s/^\s+|\s+$//g;
                next if length($s) < 3;
                print "$s\n";
            }
        '
        # Support smart quotes and code-fenced fragments.
        printf '%s\n' "$source_text" | perl -CS -ne '
            while (/“([^”]+)”|‘([^’]+)’|`([^`]+)`/g) {
                my $s = defined($1) && $1 ne "" ? $1 : defined($2) && $2 ne "" ? $2 : $3;
                $s =~ s/^\s+|\s+$//g;
                next if length($s) < 3;
                print "$s\n";
            }
        '
        # Also capture non-quoted key:value style requirements.
        printf '%s\n' "$source_text" | perl -ne '
            while (/([A-Za-z가-힣][A-Za-z가-힣0-9 _-]{1,24})\s*[:=]\s*([A-Za-z가-힣0-9 _\-]{3,80})/g) {
                my $k = $1;
                my $s = $2;
                $k =~ s/^\s+|\s+$//g;
                $s =~ s/^\s+|\s+$//g;
                next if length($s) < 3;
                next if $k =~ /^(https?|url|www)$/i;
                print "$k: $s\n";
                print "$s\n";
            }
        '
        # status/상태 문구를 비따옴표 요구사항에서도 추출.
        printf '%s\n' "$source_text" | perl -ne '
            while (/(status|상태)\s*(?:는|은|:|=)?\s*([A-Za-z0-9._-]{3,48})/ig) {
                my $k = $1;
                my $v = $2;
                $k = lc($k);
                print "$k: $v\n";
                print "$v\n";
            }
        '
        # Capture imperative payload phrases that are often unquoted.
        printf '%s\n' "$source_text" | perl -CS -ne '
            while (/(?:입력|작성|기입|붙여넣기|기록|설정)\s*(?:은|는|을|를)?\s*([A-Za-z가-힣0-9._:@#\/ _-]{3,96})/ig) {
                my $s = $1;
                $s =~ s/^\s+|\s+$//g;
                $s =~ s/[,.]$//;
                next if length($s) < 3;
                next if $s =~ /^(해|하세요|하고|후|다음)$/i;
                print "$s\n";
            }
        '
        # Prefer explicit semantic token contracts when present.
        printf '%s\n' "$source_text" | perl -CS -0777 -ne '
            while (/(?:semantic[_ -]?tokens?|의미(?:검증)?(?:토큰)?)\s*[:=]\s*\[([^\]]+)\]/ig) {
                my $raw = $1;
                for my $part (split /[,|]/, $raw) {
                    $part =~ s/^\s+|\s+$//g;
                    $part =~ s/^["'\''`“”‘’]+//;
                    $part =~ s/["'\''`“”‘’]+$//;
                    next if length($part) < 3;
                    print "$part\n";
                }
            }
            while (/(?:semantic[_ -]?tokens?|의미(?:검증)?(?:토큰)?)\s*[:=]\s*([^\n]+)/ig) {
                my $raw = $1;
                for my $part (split /[,|]/, $raw) {
                    $part =~ s/^\s+|\s+$//g;
                    $part =~ s/^["'\''`“”‘’]+//;
                    $part =~ s/["'\''`“”‘’]+$//;
                    next if length($part) < 3;
                    next if $part =~ /^(none|없음|null)$/i;
                    print "$part\n";
                }
            }
        '
        # Capture imperative multi-item payload after ":" even when not quoted.
        printf '%s\n' "$source_text" | perl -CS -0777 -ne '
            while (/(?:아래|다음)\s*(?:[0-9]+\s*줄)?[^\n:]{0,48}(?:입력|작성|기입|붙여넣기|기록|설정)[^\n:]{0,24}[:：]\s*([^\n]+)/ig) {
                my $raw = $1;
                for my $part (split /[,|]/, $raw) {
                    $part =~ s/^\s+|\s+$//g;
                    $part =~ s/^["'\''`“”‘’]+//;
                    $part =~ s/["'\''`“”‘’]+$//;
                    next if length($part) < 3 || length($part) > 96;
                    next if $part =~ /^(해|하세요|하고|후|다음)$/i;
                    next if $part =~ /^(cmd|command)\+/i;
                    print "$part\n";
                }
            }
        '
        # Capture newline bullets/numbered requirements.
        printf '%s\n' "$source_text" | perl -CS -ne '
            if (/^\s*(?:[-*]|\d+[.)])\s*(.+)$/) {
                my $s = $1;
                $s =~ s/^\s+|\s+$//g;
                $s =~ s/[,.]$//;
                if (length($s) >= 3 && length($s) <= 96) {
                    print "$s\n";
                }
            }
        '
    } | awk '!seen[$0]++'
}
