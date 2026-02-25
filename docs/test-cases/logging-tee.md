# 로그 동시 출력 테스트 케이스

## 범위

- 대상 기능: `src/logger.rs`의 파일+콘솔 동시 출력(`TeeLogWriter`)
- 검증 명령: `backup`, `run`, `restore`, `config`, `updater`
- 로그 경로: `<실행파일 경로>/logs/ardiex.log`
- updater 로그 경로: `<실행파일 경로>/logs/updater.log`

## 공통 사전조건

1. 바이너리 빌드 완료 (`cargo build`)
2. 임시 작업 디렉토리 생성 후 바이너리 복사
3. 소스/백업 디렉토리 절대 경로 사용
4. 테스트 중 `settings.json`, `logs/ardiex.log`가 실행 바이너리 경로에 생성됨

## TC-LOG-001 수동 백업 로그 동시 출력

- 목적: `backup` 실행 로그가 콘솔과 파일에 모두 기록되는지 확인
- 절차:
1. `config init`, `config add-source` 수행
2. `backup` 실행 후 stdout 캡처
3. `logs/ardiex.log` 확인
- 기대결과:
1. stdout에 `Starting manual backup` 존재
2. 로그 파일에 동일 문구 존재
3. stdout/파일 모두 `Backup completed` 존재

## TC-LOG-002 run 시작 스냅샷 동시 출력

- 목적: `run` 시작 시 `[CONFIG]` 스냅샷이 콘솔/파일 모두 출력되는지 확인
- 절차:
1. `run` 실행 후 초기 stdout 캡처
2. `logs/ardiex.log`에서 `[CONFIG]` 검색
- 기대결과:
1. stdout에 `[CONFIG]` 존재
2. 파일 로그에 `[CONFIG]` 존재

## TC-LOG-003 핫리로드 성공 로그 동시 출력

- 목적: `settings.json` 변경 후 핫리로드 성공 로그 동시 출력 확인
- 절차:
1. `run` 실행
2. `config set enable_event_driven false` 등 설정 변경
3. stdout/파일 로그에서 `[HOT-RELOAD] Applied successfully` 확인
- 기대결과:
1. 두 출력 채널 모두 성공 로그 존재

## TC-LOG-004 핫리로드 실패 로그 동시 출력

- 목적: 잘못된 설정 반영 시 실패 로그 동시 출력 확인
- 절차:
1. `run` 실행
2. `settings.json`의 `cron_schedule`을 고의로 잘못된 값으로 수정
3. stdout/파일 로그에서 `[HOT-RELOAD] Rejected invalid configuration` 확인
- 기대결과:
1. 두 출력 채널 모두 거부 로그 존재
2. 프로세스가 종료되지 않고 기존 설정 유지

## TC-LOG-005 copy 모드 run 로그 동시 출력

- 목적: copy 모드에서도 run 로그가 동일하게 양쪽으로 기록되는지 확인
- 절차:
1. `config set backup_mode copy`
2. `run` 실행 후 파일 변경 이벤트 생성
3. stdout/파일에서 watcher 관련 로그 검색
- 기대결과:
1. `File watcher started`가 두 채널에 존재
2. 백업 트리거 시 `Backup completed`가 두 채널에 존재

## TC-LOG-006 로그 회전 후 콘솔 출력 지속

- 목적: 로그 회전/압축이 발생해도 콘솔 출력이 유지되는지 확인
- 절차:
1. `config set max_log_file_size_mb 1`
2. 다량 로그 발생 (`run` + 이벤트 다수)
3. 회전 파일(`ardiex.log.<date>.gz`) 생성 여부와 콘솔 출력 확인
- 기대결과:
1. 회전 파일 생성됨
2. 회전 이후에도 콘솔에 신규 로그 계속 출력됨

## TC-LOG-007 파일 로깅 실패 시 콘솔 폴백

- 목적: 파일 로깅 초기화 실패 시 콘솔 로그가 유지되는지 확인
- 절차:
1. 로그 디렉토리 생성 실패 상황을 유도(권한 제한 환경)
2. 실행 후 stderr/stdout 확인
- 기대결과:
1. `Failed to initialize file logging` 출력
2. 이후 로그는 콘솔로 계속 출력됨

## TC-LOG-008 로그 시간 포맷 일관성

- 목적: 로컬타임 포맷이 콘솔/파일에서 동일한지 확인
- 절차:
1. 임의 명령 실행으로 로그 1개 생성
2. 콘솔과 파일의 동일 로그 라인 포맷 비교
- 기대결과:
1. `[YYYY-MM-DD HH:MM:SS.mmm LEVEL target] message` 형식 일치

## TC-LOG-009 중복 라인 검사

- 목적: 한 이벤트가 같은 채널에서 중복 기록되지 않는지 확인
- 절차:
1. `backup` 1회 수행
2. stdout과 파일 각각에서 `Starting manual backup` 개수 계산
- 기대결과:
1. 각 채널에서 1회만 기록

## TC-LOG-010 종료 시 flush 보장

- 목적: 종료 시점 로그 유실 여부 확인
- 절차:
1. `run` 실행 후 `Ctrl+C`
2. stdout/파일에서 `Ardiex backup service stopped` 확인
- 기대결과:
1. 두 채널 모두 종료 로그가 남아 있음

## TC-LOG-011 updater 로그 파일 분리

- 목적: updater 실행 시 백업 로그(`ardiex.log`)와 분리된 로그 파일(`updater.log`)에 기록되는지 확인
- 절차:
1. `updater --help` 실행
2. `logs/updater.log` 생성 여부 확인
3. `logs/ardiex.log`와 분리 저장되는지 확인
- 기대결과:
1. `updater.log`가 생성됨
2. updater 관련 로그가 `updater.log`에 기록됨

## 자동화 체크 스크립트 예시

```bash
cargo build -q
BIN=./target/debug/ardiex
ENV_DIR=$(mktemp -d /tmp/ardiex_log_case.XXXXXX)
cp "$BIN" "$ENV_DIR/ardiex"
mkdir -p "$ENV_DIR/source" "$ENV_DIR/backup"
echo "hello" > "$ENV_DIR/source/a.txt"

"$ENV_DIR/ardiex" config init >/dev/null
printf 'y\n' | "$ENV_DIR/ardiex" config add-source "$ENV_DIR/source" -b "$ENV_DIR/backup" >/dev/null
"$ENV_DIR/ardiex" backup > "$ENV_DIR/stdout.log" 2>&1

rg -q "Starting manual backup" "$ENV_DIR/stdout.log"
rg -q "Starting manual backup" "$ENV_DIR/logs/ardiex.log"
rg -q "Backup completed" "$ENV_DIR/stdout.log"
rg -q "Backup completed" "$ENV_DIR/logs/ardiex.log"
```

## 실행 기록 템플릿

| 케이스 ID | 실행일 | 실행자 | 결과(PASS/FAIL) | 비고 |
| --- | --- | --- | --- | --- |
| TC-LOG-001 |  |  |  |  |
| TC-LOG-002 |  |  |  |  |
| TC-LOG-003 |  |  |  |  |
| TC-LOG-004 |  |  |  |  |
| TC-LOG-005 |  |  |  |  |
| TC-LOG-006 |  |  |  |  |
| TC-LOG-007 |  |  |  |  |
| TC-LOG-008 |  |  |  |  |
| TC-LOG-009 |  |  |  |  |
| TC-LOG-010 |  |  |  |  |
| TC-LOG-011 |  |  |  |  |
