---
name: ardiex-assistant
description: Ardiex 증분 백업 시스템 전문가. 주기적/이벤트 기반 백업, 다중 소스/백업 경로 관리, SHA-256 증분 백업, 블록 단위 delta 백업, delta/copy 이중 모드, 글로벌/소스별 설정, 주기적 full 강제, delta 체인 검증, 백업 복구, 진행률 로깅, 파일 로깅, CLI 설정 관리, cron 스케줄링, 용량 기반 최소 백업 주기, 파일 시스템 감시(notify), Tokio 비동기 처리, GitHub Release 기반 자동 업데이트(updater), Rust 프로젝트 구조에 대한 질문에 답변하고 작업을 수행합니다.
---

# Ardiex 프로젝트 - AI 에이전트 가이드

## 프로젝트 개요

Ardiex는 Rust로 구현된 증분 백업 시스템으로, 주기적 백업과 I/O 이벤트 기반 실시간 백업을 지원합니다.

## 새로운 AI를 위한 빠른 시작 가이드

### 1. 프로젝트 구조 이해

```
ardiex/
├── src/
│   ├── main.rs          # 엔트리포인트 + 로거 초기화 + 명령어 디스패치
│   ├── cli.rs           # clap CLI 스키마
│   ├── commands/
│   │   ├── config_cmd.rs   # config 하위 커맨드 처리
│   │   ├── backup_cmd.rs   # 수동 백업 커맨드 처리
│   │   ├── restore_cmd.rs  # 복구 커맨드 처리
│   │   └── run_cmd.rs      # 서비스 실행 + 핫리로드
│   ├── config.rs        # 설정 파일 관리
│   ├── backup/
│   │   ├── mod.rs       # 백업 오케스트레이션
│   │   ├── file_ops.rs  # 파일 스캔/해시/변경감지/보관 정리
│   │   ├── metadata.rs  # metadata 동기화/이력/inc_checksum 검증
│   │   └── validation.rs # 시작 시 설정/경로/delta chain 검증
│   ├── delta.rs         # 블록 단위 delta 백업/복원
│   ├── restore.rs       # 백업 복구 관리
│   ├── watcher.rs       # 파일 시스템 감시
│   ├── logger.rs        # 파일 로깅(로컬타임, 회전/압축)
│   ├── update.rs        # GitHub release 조회/버전 비교/에셋 선택
│   ├── bin/
│   │   └── updater.rs   # 단독 업데이트 실행 파일(다운로드/교체/재시작)
│   ├── tests/           # 테스트 코드 통합 폴더
│   │   ├── backup_tests.rs    # 백업 시나리오 테스트
│   │   ├── run_cmd_tests.rs   # run 핫리로드/워처 경로 테스트
│   │   ├── logger_tests.rs    # 로그 tee writer 테스트
│   │   ├── config_tests.rs    # 설정 병합/기본값/자동 주기 계산 테스트
│   │   ├── delta_tests.rs     # delta 생성/적용/저장/로드 테스트
│   │   ├── restore_tests.rs   # restore 선택/적용/cutoff 테스트
│   │   ├── watcher_tests.rs   # watcher 이벤트 필터/디바운스 테스트
│   │   └── update_tests.rs    # 업데이트 버전/에셋 선택 테스트
│   └── editor/
│       └── settings-editor.html  # 설정 파일 웹 편집기
├── settings.json        # 실행 시 생성되는 설정 파일
├── README.md            # 프로젝트 명세 및 사용법
└── SKILL.md             # 기술 구현 가이드
```

### 2. 핵심 개념 이해

#### 증분 백업

- 첫 실행: 전체 백업 (full)
- 이후: 변경된 파일만 백업 (incremental)
- 메타데이터에 파일 해시 저장으로 변경 감지
- 블록 단위 delta 백업으로 대용량 파일 효율적 처리

#### 백업 모드

- **delta 모드**: 블록 단위 diff 백업, 주기적 + 실시간 지원
- **copy 모드**: 변경 파일 전체 복사, 주기적 + 실시간 지원
- 글로벌 설정 + 소스별 오버라이드 가능

#### 데이터 무결성

- **Delta 체인 검증**: 백업 시작 시 기존 .delta 파일 로드 검증, 손상 시 full 전환
- **Incremental 체크섬 검증**: `inc` 백업마다 `inc_checksum` 기록, 시작 시 디스크와 대조
- **주기적 full 강제**: `max_backups` 기반 자동 주기(`max_backups - 1`, 최소 1) 도달 시 full 백업
- **타임스탬프**: ms 단위로 충돌 방지

#### 글로벌/소스별 설정

- 소스별 설정이 존재하면 글로벌 오버라이드
- 소스별 오버라이드 대상 필드: `exclude_patterns`, `max_backups`, `backup_mode`, `cron_schedule`, `enable_event_driven`, `enable_periodic`
- 글로벌 전용 필드: `enable_min_interval_by_size`, `max_log_file_size_mb`
- `full_backup_interval`은 `max_backups`로 자동 계산되는 내부 값(수동 설정/저장 비활성화)
- `SourceConfig.resolve(&BackupConfig)` → `ResolvedSourceConfig`
- `config set-source <source> <key> reset`으로 초기화

#### 다중 소스/백업 경로

- 여러 소스 디렉토리 동시 관리
- 각 소스별 여러 백업 위치 지원 가능
- 백업 경로 미지정 시 `.backup` 디렉토리 사용
- **모든 경로는 절대경로 필수** (`ensure_absolute()` 검증)

#### 트리거 방식

1. **Cron 스케줄링**: crontab 표현식으로 소스별 개별 스케줄링
2. **이벤트 기반**: notify crate로 파일 변경 감지 시 실행 (delta/copy 모드 모두 지원)
3. **용량 기반 최소 주기**: ~10MB→1초, ~100MB→1분, ~1GB→1시간, 이후 GB당 1시간

#### run 핫리로드

- `run`은 `settings.json`을 주기적으로 재읽고(2초 간격) 변경을 감지하면 핫리로드 시도
- 새 설정이 유효하면 스케줄러/워처 task를 재구성하고 즉시 반영
- 새 설정이 잘못되면 기존 런타임 유지 + `[HOT-RELOAD] Rejected invalid configuration` 로그 남김
- 시작 시/핫리로드 시 설정 스냅샷을 pretty JSON으로 콘솔/로그 출력 (`[CONFIG]`)

#### 자동 업데이트

- `ardiex` 시작 시 GitHub latest release(`ardi-orrorin/ardiex`) 조회
- 최신 버전 발견 시 타깃별 에셋(`.tar.gz`/`.zip`)을 찾고 `updater`로 위임
- `updater`는 부모 종료 대기 후 실행 파일 교체 및 재실행
- 윈도우는 실행 파일 잠금 특성 때문에 `updater.exe`로 별도 교체 수행
- `ARDIEX_SKIP_UPDATE_CHECK=1`로 업데이트 재진입 루프 방지

### 3. 주요 작업별 코드 위치

#### 설정 관리 작업

- 파일: `src/config.rs`
- 함수: `ConfigManager::load_or_create()`, `save()`, `add_source()`
- 구조체: `BackupConfig`, `SourceConfig`, `BackupMode`, `ResolvedSourceConfig`
- `SourceConfig.resolve()`: 소스별 설정을 글로벌과 병합
- JSON 파싱: serde_json 사용

#### 백업 실행 작업

- 파일: `src/backup/mod.rs`, `src/backup/file_ops.rs`, `src/backup/metadata.rs`, `src/backup/validation.rs`
- 함수: `BackupManager::validate_all_sources()`, `backup_all_sources()`, `backup_source()`, `perform_backup_to_dir()`
- 시작 시 검증: `validate_all_sources()`로 metadata 이력/`inc_checksum` + delta chain + auto full interval 사전 검증, `force_full_dirs`에 결과 저장
- 해시 계산: SHA-256 사용
- Delta 백업: `find_latest_backup_file()`로 이전 백업 찾아 블록 비교
- Full 강제: 시작 시 `count_inc_since_last_full()`, `validate_delta_chain()`으로 판단
- 모드 분기: `use_delta` 플래그로 delta/copy 모드 처리
- 진행률: 10% 단위 로깅
- 용량 계산: `calculate_min_interval_by_size()`, `calculate_dir_size()`

#### Delta 백업/복원 작업

- 파일: `src/delta.rs`
- 함수: `create_delta()`, `apply_delta()`, `save_delta()`, `load_delta()`
- 4KB 블록 단위 해시 비교 및 변경 블록만 저장

#### 복구 작업

- 파일: `src/restore.rs`
- 함수: `RestoreManager::list_backups()`, `restore_to_point()`
- full 백업 기반 + inc 백업 순차 적용
- .delta 파일 자동 감지 및 복원

#### 로깅 작업

- 파일: `src/logger.rs`
- 함수: `init_file_logging_with_size()`, `init_file_logging_with_size_and_name()`, `init_console_logging()`
- 로그 위치: 실행 파일 경로의 `logs/ardiex.log`
- updater 로그 위치: 실행 파일 경로의 `logs/updater.log`
- 로컬타임 포맷: `%Y-%m-%d %H:%M:%S%.3f`
- 회전 기준: `max_log_file_size_mb`(글로벌 설정), gzip 압축, 날짜 suffix `%Y-%m-%d_%H-%M-%S`

#### 업데이트 작업

- 파일: `src/update.rs`, `src/bin/updater.rs`
- 함수(`src/update.rs`): `fetch_latest_release()`, `is_newer_version()`, `expected_release_asset_name_for_current_target()`, `find_release_asset_download_url()`
- 함수(`src/main.rs`): `maybe_delegate_to_updater()`
- updater 주요 동작: 다운로드 -> 압축해제 -> 실행 파일 교체(재시도) -> 원래 인자로 재실행

#### 파일 감시 작업

- 파일: `src/watcher.rs`
- 함수: `FileWatcher::new()`, `debounce_events()`, `should_trigger_backup()`
- 이벤트 처리: notify의 EventKind

#### CLI 명령어 처리

- 파일: `src/main.rs`, `src/cli.rs`, `src/commands/*.rs`
- 구조: `cli.rs`에서 clap Parser/Subcommand 정의, `commands`에서 실제 처리
- 명령어: config, backup, restore, run
- config 하위: init, list, add-source, remove-source, add-backup, remove-backup, set, set-source
- set: 글로벌 설정 (backup_mode, cron_schedule, enable_min_interval_by_size, max_log_file_size_mb 등)
- set-source: 소스별 설정 (cron_schedule 포함, "reset"으로 초기화 가능)
- cron 스케줄러: 소스별 개별 tokio task로 스케줄링, 용량 기반 최소 주기 적용
- 경로 검증: `ensure_absolute()`로 모든 경로 입력 절대경로 강제
- 시작 시 검증: `handle_backup()`, `handle_run()`에서 `validate_all_sources()` 호출
- `run` 시작 시 현재 설정 스냅샷 출력, 실행 중 설정 변경 핫리로드 처리

#### 릴리즈/배포 작업

- 파일: `.github/workflows/release.yml`
- 태그 빌드 시 `Cargo.toml` 버전을 태그(`vX.Y.Z`) 기준 `X.Y.Z`로 동기화 후 빌드
- 패키징 대상: `ardiex` + `updater` + `settings-editor.html`

#### 설정 에디터 작업

- 파일: `src/editor/settings-editor.html`
- `settings.json` 파일명 강제 + 로드 시 스키마 검증
- 파일 열기/전체 화면 DnD 지원
- 열기 전 저장 버튼 비활성화, 열린 파일 핸들 저장 시 덮어쓰기 저장
- JSON 미리보기 자동 갱신 및 클립보드 복사 버튼 제공

### 4. 자주 발생하는 작업 패턴

#### 새로운 기능 추가 시

1. 관련 모듈 확인 (config/backup/watcher)
2. 필요한 구조체를 config.rs에 추가
3. JSON 직렬화를 위한 serde derive 매크로 추가
4. CLI 명령어가 필요하면 `src/cli.rs`/`src/commands/*`에 반영
5. 관련 함수 구현 및 테스트

#### 버그 수정 시

1. 에러 로그 확인으로 문제 위치 파악
2. anyhow의 context()로 추가 정보 수집
3. 관련 단위 테스트 작성으로 재현
4. 수정 후 통합 테스트 실행

#### 성능 최적화 시

1. 병렬 처리 가능한 부분 확인 (tokio::join!)
2. 대용량 파일 처리 시 스트리밍 사용
3. 불필요한 파일 해시 계산 제거
4. 디바운싱으로 중복 작업 방지

### 5. 디버깅 팁

#### 로그 활성화

```bash
RUST_LOG=debug ./ardiex run
```

#### 설정 파일 확인

- 위치: 실행 파일과 동일한 경로의 settings.json
- 없으면 기본값으로 자동 생성됨

#### 백업 실패 시 확인사항

1. 소스 디렉토리 존재 여부
2. 백업 디렉토리 쓰기 권한
3. 디스크 공간 충분 여부
4. exclude_patterns에 의한 제외 여부

### 6. 테스트 방법

#### 단위 테스트 실행

```bash
cargo test --all-targets
```

- 테스트 코드는 `src/tests/*.rs`에 통합되어 있으며 각 모듈에서 `#[path]`로 로딩됩니다.
- 기능 추가/코드 수정 시 관련 테스트 코드를 반드시 수정 또는 추가하고, 테스트 실행으로 검증해야 합니다.
- 테스트는 실패 경로를 우선으로 작성합니다(설정값 오류, 경로 오류, 데이터 손상, 파싱 실패, 핫리로드 거부 등).

#### 통합 테스트

```bash
cargo test --test integration
```

- TDD 상세 케이스 문서: `docs/test-cases/tdd-test-plan.md`

#### 수동 백업 테스트

```bash
./ardiex backup
```

### 7. 주의사항

#### 보안

- 설정 파일에 민감 정보 포함 주의
- 백업 경로 권한 확인
- 해시 충돌은 무시해도 됨 (SHA-256 사용)

#### 성능

- 대용량 디렉토리는 초기 해시 계산에 시간 소요
- 네트워크 백업 경로는 타임아웃 설정 필요
- 파일 변경이 잦은 디렉토리는 디바운싱 필수

#### 호환성

- Windows/Mac/Linux 모두 지원 (notify crate)
- 경로 구분자: std::path::Path 사용
- 파일 권한: 플랫폼별 차이 고려

### 8. 확장 아이디어

#### 추가 기능

- 압축 백업 (zip, tar.gz)
- 암호화 백업
- 클라우드 저장소 연동 (AWS S3, Google Drive)
- 백업 스케줄링 (cron-like)
- 백업 리포트 및 통계

#### 개선 사항

- ~~백업 속도 최적화 (rsync 알고리즘)~~ (delta 백업으로 구현)
- 실시간 동기화 (bidirectional sync)
- 버전 관리 (Git-like)
- 중복 제거 (deduplication)
- ~~증분 복원 기능~~ (구현 완료)
- ~~delta/copy 이중 모드~~ (구현 완료)
- ~~글로벌/소스별 설정~~ (구현 완료)
- ~~주기적 full 강제~~ (구현 완료)
- ~~delta 체인 검증~~ (구현 완료)
- ~~진행률 로깅~~ (구현 완료)
- ~~cron 스케줄링~~ (구현 완료)
- ~~용량 기반 최소 백업 주기~~ (구현 완료)

## 자주 묻는 질문

### Q: 설정 파일이 없으면 어떻게 되나요?

A: 기본값으로 settings.json이 자동 생성됩니다.

### Q: 백업 중 프로그램이 종료되면 어떻게 되나요?

A: 다음 실행 시 메타데이터를 보고 이어서 진행합니다.

### Q: 대용량 파일은 어떻게 처리되나요?

A: 스트림으로 chunk 단위로 읽어 메모리 사용량을 최소화합니다.

### Q: 네트워크 드라이브도 백업 가능한가요?

A: 네, 단 타임아웃과 재시도 로직이 필요합니다.
