# Ardiex 백업 프로그램 명세

## 개요

주기적 또는 I/O 이벤트 기반으로 동작하는 증분 백업 프로그램 (Rust 구현)

## 주요 기능

### 1. 백업 방식

- **증분 백업**: 초기 전체 백업 후 변경된 파일만 백업
- **다중 소스 지원**: 여러 소스 디렉토리 동시 관리
- **다중 백업 경로**: 각 소스별 여러 백업 위치 지원
- **두 가지 백업 모드**:
  - **delta**: 블록 단위 diff 백업 (주기적 + 실시간 지원)
  - **copy**: 변경 파일 전체 복사 (주기적 + 실시간 지원)
- **주기적 full 강제**: `max_backups` 기반 자동 주기(`max_backups - 1`, 최소 1) 도달 시 full 백업
- **Delta 체인 검증**: 백업 시작 시 기존 delta 파일 무결성 검증, 손상 시 full 전환
- **메타데이터 이력 검증**: 백업 시작 시 `metadata.json`의 `backup_history`와 실제 백업 디렉토리 전체 일치 여부 검증
- **증분 체크섬 검증**: `inc` 백업마다 체크섬(`inc_checksum`)을 기록하고 시작 시 디스크와 대조
- **글로벌/소스별 설정**: 소스별 설정이 글로벌 설정을 오버라이드
- **시작 시 검증**: 프로그램 시작 시 설정 파일 전체 유효성 검사
- **`run` 핫리로드**: 실행 중 `settings.json` 변경 감지 후 런타임 작업(스케줄러/워처) 재구성
- **설정 스냅샷 출력**: `run` 시작 시 현재 설정을 pretty JSON으로 콘솔/로그에 출력
- **로그 회전/압축**: `max_log_file_size_mb` 초과 시 gzip 압축 + 날짜 suffix로 자동 회전
- **자동 업데이트**: 실행 시 GitHub Release 최신 버전 조회 후 신규 버전이 있으면 `updater` 바이너리로 교체 수행

### 2. 트리거 방식

- **Cron 스케줄링**: crontab 표현식으로 백업 주기 설정 (글로벌/소스별)
- **I/O 이벤트 기반**: 파일 시스템 변경 감지 시 즉시 실행 (delta/copy 모드 모두 지원)
- **용량 기반 최소 주기**: 소스 디렉토리 크기에 따라 최소 백업 간격 자동 적용

### 3. 용량 기반 최소 백업 주기

| 소스 용량 | 최소 주기                    |
| --------- | ---------------------------- |
| ~10MB     | 1초                          |
| ~100MB    | 1분                          |
| ~1GB      | 1시간                        |
| 1GB 초과  | GB당 1시간 (ex: 3GB → 3시간) |

> `enable_min_interval_by_size: false`로 비활성화 가능

### 4. Cron 스케줄링

Ardiex는 **6필드 cron 표현식** (초 포함)을 사용합니다.

```
┌──────── 초 (0-59)
│ ┌────── 분 (0-59)
│ │ ┌──── 시 (0-23)
│ │ │ ┌── 일 (1-31)
│ │ │ │ ┌ 월 (1-12)
│ │ │ │ │ ┌ 요일 (0-6, 0=일요일)
│ │ │ │ │ │
* * * * * *
```

#### 자주 쓰는 예시

| 표현식           | 설명               |
| ---------------- | ------------------ |
| `0 0 * * * *`    | 매시 정각 (기본값) |
| `0 */30 * * * *` | 30분마다           |
| `0 */5 * * * *`  | 5분마다            |
| `0 0 */2 * * *`  | 2시간마다          |
| `0 0 9 * * *`    | 매일 오전 9시      |
| `0 0 0 * * *`    | 매일 자정          |
| `0 0 9 * * 1-5`  | 평일 오전 9시      |
| `0 0 0 * * 0`    | 매주 일요일 자정   |
| `0 0 0 1 * *`    | 매월 1일 자정      |

#### 특수 문자

| 문자  | 의미    | 예시                             |
| ----- | ------- | -------------------------------- |
| `*`   | 모든 값 | `* * * * * *` (매초)             |
| `*/N` | N 간격  | `*/10 * * * * *` (10초마다)      |
| `N-M` | 범위    | `0 0 9-17 * * *` (9시~17시 매시) |
| `N,M` | 목록    | `0 0,30 * * * *` (0분, 30분)     |

> **참고**: 일반 crontab은 5필드(분부터)이지만, Ardiex는 **초 필드가 맨 앞에** 추가됩니다.
>
> 5필드 표현식 확인: [crontab.guru](https://crontab.guru) (앞에 `0 ` 추가하여 사용)

## 설정 파일 (settings.json)

### 위치

- 실행 파일과 동일한 경로
- 없으면 기본값으로 자동 생성
- 절대 경로로 설정

### 구조

```json
{
  "sources": [
    {
      "source_dir": "/home/user/documents",
      "backup_dirs": ["/backup/documents", "/mnt/external/documents"],
      "enabled": true,
      "exclude_patterns": ["*.cache"],
      "max_backups": 5,
      "backup_mode": "copy",
      "cron_schedule": "0 */5 * * * *",
      "enable_event_driven": false,
      "enable_periodic": true
    },
    {
      "source_dir": "/home/user/photos",
      "backup_dirs": ["/backup/photos"],
      "enabled": true
    }
  ],
  "enable_periodic": true,
  "enable_event_driven": true,
  "exclude_patterns": ["*.tmp", "*.log", ".git/*"],
  "max_backups": 10,
  "max_log_file_size_mb": 20,
  "backup_mode": "delta",
  "cron_schedule": "0 0 * * * *",
  "enable_min_interval_by_size": true,
  "metadata": {
    "/home/user/documents": {
      "last_full_backup": "2024-02-21T10:00:00Z",
      "last_backup": "2024-02-21T11:30:00Z",
      "file_hashes": {
        "file1.txt": "sha256_hash...",
        "subdir/file2.pdf": "sha256_hash..."
      },
      "backup_history": [
        {
          "backup_name": "full_20240221_100000123",
          "backup_type": "full",
          "created_at": "2024-02-21T10:00:00Z",
          "files_backed_up": 120,
          "bytes_processed": 345678901
        },
        {
          "backup_name": "inc_20240221_110000456",
          "backup_type": "inc",
          "created_at": "2024-02-21T11:00:00Z",
          "files_backed_up": 8,
          "bytes_processed": 1234567,
          "inc_checksum": "sha256_inc_snapshot_hash..."
        }
      ]
    }
  }
}
```

### 시작 시 검증 항목

프로그램 시작(`backup`, `run`) 시 다음 항목을 자동 검증합니다:

- 글로벌 `cron_schedule` 유효성
- 글로벌 `max_backups > 0`, `max_log_file_size_mb > 0`
- 소스 중복 여부
- 소스/백업 경로: 절대경로, 존재 여부, 디렉토리 여부
- 소스 == 백업 동일 경로 금지, 백업 중복 검사
- 소스별 오버라이드 값 검증 (`max_backups`, `cron_schedule`)
- 메타데이터 이력(`backup_history`)과 실제 백업 디렉토리 전체 일치 여부 검증
- 메타데이터 `inc_checksum`과 실제 `inc` 백업 디렉토리 체크섬 일치 여부 검증 (불일치 시 full 강제)
- Delta chain 무결성 검증, 자동 계산된 full 주기 도달 시 full 강제

### 백업 경로 규칙

- `backup_dirs`가 비어있으면: `{source_dir}/.backup` 사용
- `backup_dirs`에 값이 있으면: 모든 경로에 순차적으로 백업

## CLI 명령어

### 설정 관리

```bash
ardiex config init                    # 기본 설정 파일 생성
ardiex config list                    # 현재 설정 조회
ardiex config add-source <path>       # 새 소스 추가
ardiex config remove-source <path>    # 소스 제거
ardiex config add-backup <source> <backup_path>  # 소스에 백업 경로 추가
ardiex config remove-backup <source> <backup_path>  # 소스에서 백업 경로 제거
ardiex config set <key> <value>       # 글로벌 설정 변경
ardiex config set-source <source> <key> <value>  # 소스별 설정 변경
ardiex config set-source <source> <key> reset     # 소스별 설정 초기화 (글로벌로 폴백)
```

### 백업 실행

```bash
ardiex backup                         # 수동 백업 실행
ardiex run                            # 백업 서비스 시작 (주기적+이벤트)
```

### 복구

```bash
ardiex restore <backup_dir> <target_dir> --list          # 백업 목록 조회
ardiex restore <backup_dir> <target_dir>                  # 최신 시점으로 복구
ardiex restore <backup_dir> <target_dir> --point <timestamp>  # 특정 시점으로 복구
```

## 사용법

### 1. 빌드

```bash
cargo build --release
```

### 2. 초기 설정

```bash
# 설정 파일 초기화
./ardiex config init

# 백업할 소스 디렉토리 추가
./ardiex config add-source /home/user/documents --backup /backup/documents

# 여러 백업 경로 지정 가능
./ardiex config add-source /home/user/photos --backup /backup/photos --backup /mnt/external/photos

# 설정 확인
./ardiex config list
```

### 3. 수동 백업

```bash
# 즉시 백업 실행
./ardiex backup

# 출력 예시:
# Backup completed: 15 files to "/backup/documents" (23.45 MB in 1250 ms)
```

### 4. 자동 백업 서비스 실행

```bash
# 백업 서비스 시작 (Ctrl+C로 종료)
./ardiex run

# cron 스케줄 기반 백업과 파일 변경 감지 백업이 동시에 실행됨
# 시작 시 현재 설정값을 pretty JSON으로 출력
# [CONFIG] { "phase": "startup", "config": { ... } }
# 실행 중 settings.json 변경 시 핫리로드 로그 출력
# [HOT-RELOAD] Detected settings.json change ...
# [HOT-RELOAD] Applied successfully ...
```

### 5. 설정 변경

```bash
# 글로벌 설정
./ardiex config set enable_periodic false
./ardiex config set enable_event_driven false
./ardiex config set max_backups 20
./ardiex config set max_log_file_size_mb 50  # 로그 파일 50MB마다 회전
./ardiex config set backup_mode delta          # delta 또는 copy
./ardiex config set cron_schedule "0 */30 * * * *"  # 30분마다 (초 분 시 일 월 요일)
./ardiex config set enable_min_interval_by_size false  # 용량 기반 최소 주기 비활성화
# full_backup_interval은 max_backups로 자동 계산되며 수동 설정할 수 없음

# 소스별 설정 (글로벌 오버라이드)
./ardiex config set-source /home/user/documents backup_mode copy
./ardiex config set-source /home/user/documents max_backups 5
./ardiex config set-source /home/user/documents exclude_patterns "*.cache,*.tmp"
./ardiex config set-source /home/user/documents cron_schedule "0 */5 * * * *"  # 5분마다

# 소스별 설정 초기화 (글로벌로 폴백)
./ardiex config set-source /home/user/documents backup_mode reset
./ardiex config set-source /home/user/documents cron_schedule reset
```

### 설정 우선순위

소스별 설정이 존재하면 글로벌 설정보다 우선 적용됩니다.

| 설정 키                | 글로벌           | 소스별 (Optional)  |
| ---------------------- | ---------------- | ------------------ |
| `exclude_patterns`     | `["*.tmp", ...]` | 지정 시 오버라이드 |
| `max_backups`          | `10`             | 지정 시 오버라이드 |
| `backup_mode`          | `"delta"`        | 지정 시 오버라이드 |
| `cron_schedule`        | `"0 0 * * * *"`  | 지정 시 오버라이드 |
| `enable_event_driven`  | `true`           | 지정 시 오버라이드 |
| `enable_periodic`      | `true`           | 지정 시 오버라이드 |

> `full_backup_interval`은 사용자 입력값이 아니라 `max_backups`로부터 자동 계산되는 내부 값입니다. `settings.json`과 설정 에디터에는 저장/노출되지 않습니다.

### 6. 백업 관리

```bash
# 백업 디렉토리 구조 예시:
# /backup/documents/
# ├── full_20240221_100000123/  # 전체 백업 (ms 단위 타임스탬프)
# ├── inc_20240221_110000456/   # 증분 백업 (delta 또는 copy)
# ├── inc_20240221_120000789/
# └── metadata.json             # 백업 메타데이터
```

### 7. 백업 복구

```bash
# 백업 목록 조회
./ardiex restore /backup/documents /home/user/restored --list
# 출력 예시:
# [FULL] 20240221_100000 (full_20240221_100000)
# [INC ] 20240221_110000 (inc_20240221_110000)
# [INC ] 20240221_120000 (inc_20240221_120000)

# 최신 시점으로 전체 복구 (full + 모든 inc 적용)
./ardiex restore /backup/documents /home/user/restored

# 특정 시점으로 복구
./ardiex restore /backup/documents /home/user/restored --point 20240221_110000
```

## 증분 백업 알고리즘

### Delta 모드 프로세스

1. **파일 해시 계산**: SHA-256으로 각 파일의 해시 계산
2. **변경 감지**: 이전 해시와 비교하여 변경된 파일 식별
3. **Delta 체인 검증**: 기존 delta 파일 무결성 확인, 손상 시 full 전환
4. **Full 강제 확인**: 자동 계산된 full 주기 도달 시 full 백업 강제
5. **Delta 백업**: 이전 백업 파일이 있으면 4KB 블록 단위 비교 후 `.delta` 파일 저장
6. **Fallback 복사**: 이전 파일이 없으면 변경 파일 전체 복사
7. **메타데이터 업데이트**: 파일 해시/이력 정보 저장 (`inc`는 `inc_checksum` 포함)

### Copy 모드 프로세스

1. **파일 해시 계산**: SHA-256으로 각 파일의 해시 계산
2. **변경 감지**: 이전 해시와 비교하여 변경된 파일 식별
3. **파일 복사**: 변경된 파일 전체를 백업 디렉토리에 복사
4. **메타데이터 업데이트**: 파일 해시/이력 정보 저장 (`inc_checksum` 포함)

> **참고**: copy 모드도 실시간(이벤트 기반) 백업을 지원합니다.

### 백업 디렉토리 구조

```
backup/
├── full_20240221_100000123/  # 전체 백업 (ms 단위 타임스탬프)
├── inc_20240221_110000456/   # 증분 백업
├── inc_20240221_120000789/
└── metadata.json             # 백업 메타데이터
```

### 진행률 로깅

백업 및 복구 시 10% 단위로 진행률이 로그에 기록됩니다.

```
[2026-02-21 12:30:00.123 INFO ardiex::backup] Backup progress: 10% (10/100 files)
[2026-02-21 12:30:01.456 INFO ardiex::backup] Backup progress: 20% (20/100 files)
...
[2026-02-21 12:30:05.000 INFO ardiex::restore] Restore progress: 50% - Applied backup 'full_20240221_100000123': 50 files restored
```

## 설정 에디터 (Web)

- 파일: `src/editor/settings-editor.html`
- `settings.json` 파일만 불러오기 허용 (파일 열기 + 화면 전체 Drag & Drop)
- 불러오기 시 JSON 파싱 + 스키마 검증 실패하면 즉시 에러
- 저장 버튼은 파일을 불러온 뒤에만 활성화
- File System Access API 지원 브라우저에서 열린 파일에 덮어쓰기 저장
- JSON 미리보기 토글 + "미리보기 JSON 복사" 버튼 제공
- 폼 값 변경 시 JSON 미리보기 자동 갱신
- 파일 드롭 관련 안내 문구를 "파일 불러오기"로 통일
- `full_backup_interval` 입력 UI 제거 (자동 계산/저장 제외)

## 로그 파일 관리

- 로그 파일: 실행 파일 경로의 `logs/ardiex.log`
- updater 로그 파일: 실행 파일 경로의 `logs/updater.log`
- 로그 시간: 로컬 타임(`%Y-%m-%d %H:%M:%S%.3f`)
- 회전 기준: 글로벌 설정 `max_log_file_size_mb` (기본 20MB)
- 회전 시 파일명 suffix: `%Y-%m-%d_%H-%M-%S`
- 회전된 로그는 gzip으로 자동 압축, 최대 30개 보관
- 로그는 파일 저장과 콘솔 출력이 동시에 수행됨
- 상세 테스트 케이스: `docs/test-cases/logging-tee.md`
- TDD 테스트 케이스 계획: `docs/test-cases/tdd-test-plan.md`

## 자동 업데이트

- 저장소: `ardi-orrorin/ardiex` GitHub Release 기준
- 실행 흐름:
1. `ardiex` 시작 시 latest release 조회
2. 현재 버전보다 최신 태그가 있으면 현재 타깃(OS/ARCH)에 맞는 에셋 탐색
3. 같은 경로의 `updater`(윈도우는 `updater.exe`)를 실행하고 `ardiex`는 종료
4. `updater`가 에셋 다운로드/압축 해제 후 `ardiex` 실행 파일 교체
5. 원래 인자로 `ardiex` 재실행
- 루프 방지: `ARDIEX_SKIP_UPDATE_CHECK=1` 환경변수로 재시작 프로세스의 재검사 차단
- 지원 에셋명:
  - `ardiex-linux-amd64.tar.gz`
  - `ardiex-linux-arm64.tar.gz`
  - `ardiex-macos-amd64.tar.gz`
  - `ardiex-macos-arm64.tar.gz`
  - `ardiex-windows-amd64.zip`
- 윈도우 교체 전략: 부모 프로세스 종료 대기 + 파일 교체 재시도

### 릴리즈 파이프라인 연동

- 워크플로우: `.github/workflows/release.yml`
- 태그 푸시(`vX.Y.Z`) 시 build 단계 시작 전에 `Cargo.toml`의 `version`을 태그 버전(`X.Y.Z`)으로 동기화
- 릴리즈 아카이브에 `ardiex`와 `updater`(윈도우는 `ardiex.exe`, `updater.exe`)를 함께 패키징

## 기술 스택

- **언어**: Rust
- **비동기 런타임**: Tokio
- **파일 시스템 감시**: notify
- **시간 처리**: chrono
- **JSON 처리**: serde + serde_json
- **CLI**: clap
- **로깅**: log + env_logger
- **에러 처리**: anyhow
- **해시 계산**: sha2 (SHA-256)
- **로그 파일 회전**: file-rotate
- **Cron 스케줄링**: cron
- **디렉토리 탐색**: walkdir
- **업데이트 통신**: reqwest (blocking + rustls)
- **업데이트 압축 해제**: tar + zip + flate2

## 주요 의존성

```toml
tokio = { version = "1.0", features = ["full"] }
notify = "6.1"
chrono = { version = "0.4", features = ["serde"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
clap = { version = "4.0", features = ["derive"] }
log = "0.4"
env_logger = "0.10"
anyhow = "1.0"
sha2 = "0.10"
file-rotate = "0.7"
walkdir = "2.5"
mimalloc = "0.1"
reqwest = { version = "0.12", default-features = false, features = ["blocking", "json", "rustls-tls"] }
tar = "0.4"
zip = "2.2"
flate2 = "1.0"
```

## Release 프로필 최적화

```toml
[profile.release]
lto = "fat"            # 전체 LTO로 크로스 크레이트 최적화 극대화
codegen-units = 1      # 단일 코드 생성 단위로 최적화 극대화
panic = "abort"        # unwinding 코드 제거, 바이너리 축소
strip = true           # 디버그 심볼 제거
opt-level = 3          # 최대 성능 최적화
overflow-checks = false  # 오버플로 검사 제거로 성능 향상
```

| 설정              | 값        | 효과                                |
| ----------------- | --------- | ----------------------------------- |
| `lto`             | `"fat"`   | 전체 LTO로 최대 바이너리 최적화     |
| `codegen-units`   | `1`       | 단일 코드 생성 단위로 최적화 극대화 |
| `panic`           | `"abort"` | unwinding 코드 제거, 바이너리 축소  |
| `strip`           | `true`    | 디버그 심볼 제거                    |
| `opt-level`       | `3`       | 최대 성능 최적화                    |
| `overflow-checks` | `false`   | 오버플로 검사 제거로 성능 향상      |

## 모듈 구조

1. **main.rs** - 엔트리포인트 + 로거 초기화 + 명령어 디스패치
2. **cli.rs** - Clap CLI 스키마 (`config/backup/restore/run`)
3. **commands/config_cmd.rs** - 설정 관리 커맨드 처리
4. **commands/backup_cmd.rs** - 수동 백업 커맨드 처리
5. **commands/restore_cmd.rs** - 복구 커맨드 처리
6. **commands/run_cmd.rs** - 서비스 실행 + 주기/이벤트 트리거 + 핫리로드
7. **config.rs** - 설정 파일 로드/저장 + 기본값 + 소스/글로벌 병합
8. **backup/mod.rs** - 백업 오케스트레이션 + full/inc 결정
9. **backup/file_ops.rs** - 파일 스캔/해시/변경감지/보관 정리
10. **backup/metadata.rs** - metadata 로드/동기화/이력 검증
11. **backup/validation.rs** - 시작 시 경로/설정/delta chain 검증
12. **delta.rs** - 블록 단위 delta 백업/복원
13. **restore.rs** - 백업 복구 관리
14. **watcher.rs** - 파일 시스템 감시
15. **logger.rs** - 파일 로깅(로컬타임, 회전/압축, 파일+콘솔 tee)
16. **update.rs** - GitHub release 조회/버전 비교/타깃 에셋 선택
17. **bin/updater.rs** - 단독 업데이트 실행 파일(다운로드/교체/재시작)
18. **editor/settings-editor.html** - 설정 파일 웹 편집기
19. **tests/** - 테스트 코드 통합 폴더 (`backup/run_cmd/logger/config/delta/restore/watcher/update` 테스트)

## 테스트 코드 구조

- 단위/모듈 테스트 코드는 `src/tests`에 통합 관리
- 각 실제 모듈(`backup/mod.rs`, `commands/run_cmd.rs`, `logger.rs`, `config.rs`, `delta.rs`, `restore.rs`, `watcher.rs`)에서 `#[path = "..."]`로 테스트 파일 연결
- 기능 추가/코드 수정 시 관련 테스트 코드를 반드시 수정 또는 추가하고, 변경 후 테스트 실행으로 검증해야 함
- 테스트 추가 우선순위: 성공 경로보다 실패 경로(잘못된 설정/입력/파일 손상/경로 오류/핫리로드 거부)를 먼저 커버
- 현재 테스트 파일:
  - `src/tests/backup_tests.rs`
  - `src/tests/run_cmd_tests.rs`
  - `src/tests/logger_tests.rs`
  - `src/tests/config_tests.rs`
  - `src/tests/delta_tests.rs`
  - `src/tests/restore_tests.rs`
  - `src/tests/watcher_tests.rs`
  - `src/tests/update_tests.rs`
