---
name: ardiex
description: Ardiex 증분 백업 시스템 작업 전용. 주기적/이벤트 기반 백업, 다중 소스/백업 경로, SHA-256 증분 백업, 블록 단위 delta 백업, delta/copy 이중 모드, 글로벌/소스별 설정, 주기적 full 강제, delta 체인 검증, 백업 복구, 진행률 로깅, 파일 로깅, CLI 설정 관리, cron 스케줄링, 용량 기반 최소 백업 주기, JSON 설정 파일, 파일 시스템 감시(notify), Tokio 비동기 처리가 필요한 작업에 사용.
---

# Ardiex 백업 프로그램 기술 명세

## 핵심 기술 스택

### 1. Rust 비동기 프로그래밍

- **Tokio**: 비동기 런타임으로 cron 스케줄링 및 이벤트 처리
- **Async/Await**: 동시성 처리 (백업 실행, 파일 감시, CLI 처리)
- **cron crate**: crontab 표현식 파싱 및 다음 실행 시간 계산 (기본 6필드: 초 분 시 일 월 요일)

### 2. 파일 시스템 감시

- **notify crate**: 크로스플랫폼 파일 시스템 이벤트 감지
- **이벤트 종류**: Create, Modify, Remove, Rename
- **디바운싱**: 연속적인 파일 변경에 대한 중복 백업 방지

### 3. 증분 백업 알고리즘

```rust
// 핵심 로직
fn calculate_file_hash(path: &Path) -> Result<String>
fn detect_changed_files(source: &Path, metadata: &Metadata) -> Result<Vec<PathBuf>>
fn perform_incremental_backup(source: &Path, backup_dir: &Path, changed_files: Vec<PathBuf>) -> Result<()>
```

### 4. 블록 단위 Delta 백업

- **블록 크기**: 4KB 단위로 파일 분할
- **블록 해시 비교**: 이전 백업과 현재 파일의 각 블록 SHA-256 해시 비교
- **Delta 생성 조건**: delta 모드 + 이전 백업 파일이 존재할 때 `.delta` 생성
- **Fallback**: 이전 파일이 없으면 전체 파일 복사
- **복원**: 원본 파일 + delta 블록 병합으로 복원

### 4-1. 백업 모드 (delta / copy)

- **delta 모드**: 블록 단위 diff 백업, 주기적 + 실시간 지원
- **copy 모드**: 변경 파일 전체 복사, 주기적 + 실시간 지원
- **글로벌/소스별 설정**: 소스별 설정이 글로벌 오버라이드

### 4-2. Delta 체인 무결성 검증

- 백업 시작 시 기존 inc 디렉토리의 .delta 파일을 모두 로드 검증
- 손상 감지 시 자동으로 full 백업으로 전환
- 자동 계산된 `full_backup_interval` 도달 시에도 full 강제

### 4-3. 진행률 로깅

- 백업/복구 시 10% 단위로 진행률 로그 기록
- 파일 수 기반 비율 계산

### 4-4. Cron 스케줄링

- 글로벌 `cron_schedule` + 소스별 `cron_schedule` 오버라이드
- `cron` crate로 파싱, `schedule.upcoming(Utc).next()`로 다음 실행 시간 계산
- 소스별 개별 tokio task로 스케줄링

### 4-5. 용량 기반 최소 백업 주기

- 소스 디렉토리 크기 재귀적 계산
- ~10MB: 1초, ~100MB: 1분, ~1GB: 1시간, 이후 GB당 1시간
- `enable_min_interval_by_size`로 on/off
- cron 트리거 후 최소 주기 미달 시 대기

### 4-6. 메타데이터 이력/체크섬 검증

- `backup_history`는 디스크의 `full_*/inc_*` 디렉토리와 1:1로 일치해야 함
- `inc` 백업은 `inc_checksum`을 metadata에 기록
- 프로그램 시작(`backup`, `run`) 시 `inc_checksum`과 실제 `inc` 디렉토리 체크섬을 검증
- 불일치 감지 시 해당 백업 경로를 `force_full`로 표시하여 다음 백업을 full로 강제

```rust
// delta.rs 핵심 함수
pub fn create_delta(original: &Path, new: &Path) -> Result<DeltaFile>
pub fn apply_delta(original: &Path, delta: &DeltaFile, output: &Path) -> Result<()>
pub fn save_delta(delta: &DeltaFile, path: &Path) -> Result<()>
pub fn load_delta(path: &Path) -> Result<DeltaFile>
```

### 5. 백업 복구

- **타임스탬프**: ms 단위 (`%Y%m%d_%H%M%S%3f`)
- **복구 시 진행률 로깅**: 백업 단위 + 파일 단위 10% 로깅

- **시점별 복구**: full 백업 + inc 백업들을 시간순으로 적용
- **Delta 복원**: .delta 파일을 이전 복원 파일에 적용
- **백업 목록 조회**: 사용 가능한 복원 시점 확인

```rust
// restore.rs 핵심 함수
pub fn list_backups(backup_dir: &Path) -> Result<Vec<BackupEntry>>
pub fn restore_to_point(backup_dir: &Path, target: &Path, point: Option<&str>) -> Result<usize>
```

### 6. 파일 로깅

- **로그 위치**: 실행 파일 경로의 `logs/ardiex.log`
- **시간 포맷**: 로컬타임 `%Y-%m-%d %H:%M:%S%.3f`
- **회전 정책**: `max_log_file_size_mb` 초과 시 회전
- **회전 파일명**: 날짜 suffix `%Y-%m-%d_%H-%M-%S`
- **압축**: 회전 시 gzip 압축(`Compression::OnRotate(1)`), 최대 30개 보관
- **기록 내용**: 백업 시작/완료, 복구, 에러, delta 정보, 핫리로드 상태

### 7. SHA-256 해시 계산

- **std::fs::File**: 파일 읽기
- **sha2::Sha256**: 해시 계산
- **std::io::Read**: 스트림으로 대용량 파일 처리

### 8. JSON 설정 관리

- **serde**: 직렬화/역직렬화
- **serde_json**: JSON 파일 입출력
- **실행 파일 경로**: std::env::current_exe()로 설정 파일 위치 결정
- **글로벌/소스별 설정**: `SourceConfig.resolve(&BackupConfig)` → `ResolvedSourceConfig`
- **소스별 설정 필드**: `Option<T>`로 선언, `#[serde(default, skip_serializing_if = "Option::is_none")]`
- **cron_schedule**: 글로벌 + 소스별 오버라이드, `cron::Schedule::from_str()`로 검증
- **로그 회전 설정**: 글로벌 `max_log_file_size_mb` (기본 20)
- **소스별 설정 오버라이드**: `exclude_patterns`, `max_backups`, `backup_mode`, `cron_schedule`, `enable_event_driven`, `enable_periodic`
- **자동 계산 필드**: `full_backup_interval`은 `max_backups` 기반으로 내부 계산되며 설정 파일/에디터에 저장되지 않음
- **메타데이터 확장 필드**: `backup_history[].inc_checksum` (incremental 백업 체크섬)

### 9. run 런타임 핫리로드

- `run` 실행 중 2초 간격으로 `settings.json` 변경 감지
- 변경 감지 시 `[HOT-RELOAD]` 로그를 남기고 새 설정 검증
- 유효하면 스케줄러/워처 task를 재생성하여 즉시 반영
- 무효하면 기존 설정을 유지하고 거부 로그 남김
- 시작 시/핫리로드 시 `[CONFIG]` pretty JSON 스냅샷 출력

## 구현 패턴

### 1. 설정 관리 패턴

```rust
pub struct ConfigManager {
    config_path: PathBuf,
    config: BackupConfig,
}

impl ConfigManager {
    pub fn load_or_create() -> Result<Self>
    pub fn save(&mut self) -> Result<()>
    pub fn add_source(&mut self, source_dir: PathBuf, backup_dirs: Vec<PathBuf>) -> Result<()>
    pub fn remove_source(&mut self, source_dir: &Path) -> Result<()>
}
```

### 2. 백업 관리자 패턴

```rust
// src/backup/mod.rs
pub struct BackupManager {
    config: BackupConfig,
    force_full_dirs: HashMap<PathBuf, bool>,  // 시작 시 검증 결과
}

impl BackupManager {
    pub fn validate_all_sources(&mut self)  // 프로그램 시작 시 delta chain + auto full interval 검증
    pub async fn backup_all_sources(&mut self) -> Result<Vec<BackupResult>>
    async fn backup_source(source, backup_dirs, resolved, force_full_dirs) -> Result<Vec<BackupResult>>
    async fn perform_backup_to_dir(source, backup, exclude, max, mode, force_full) -> Result<BackupResult>
    fn count_inc_since_last_full(backup_dir: &Path) -> usize
    fn validate_delta_chain(backup_dir: &Path) -> bool
    fn find_latest_backup_file(backup_dir: &Path, relative: &Path) -> Option<PathBuf>
}
```

- `src/backup/file_ops.rs`: 파일 스캔/변경감지/해시/보관 정리
- `src/backup/metadata.rs`: metadata 로드/동기화/backup_history + inc_checksum 검증
- `src/backup/validation.rs`: 시작 시 설정/경로/delta chain 검증

### 3. 파일 감시자 패턴

```rust
pub struct FileWatcher {
    _watchers: Vec<RecommendedWatcher>,
    _backup_tx: tokio_mpsc::Sender<()>,
}

impl FileWatcher {
    pub fn new(paths: Vec<PathBuf>, backup_tx: Sender<()>, debounce: Duration) -> Result<Self>
    fn debounce_events(rx: Receiver<Event>, backup_tx: Sender<()>, debounce: Duration)
    fn should_trigger_backup(event: &Event) -> bool
}
```

### 4. 복구 관리자 패턴

```rust
pub struct RestoreManager;

impl RestoreManager {
    pub fn list_backups(backup_dir: &Path) -> Result<Vec<BackupEntry>>
    pub fn restore_to_point(backup_dir: &Path, target: &Path, point: Option<&str>) -> Result<usize>
    fn select_backups(backups: &[BackupEntry], point: Option<&str>) -> Result<Vec<&BackupEntry>>
    fn apply_backup(backup: &BackupEntry, target: &Path, backup_root: &Path) -> Result<usize>
}
```

## 에러 처리 전략

### 1. anyhow 사용

- 컨텍스트 정보 포함한 에러 체인
- Result<T> 타입으로 에러 전파

### 2. 주요 에러 케이스

- 파일 접근 권한 없음
- 디스크 공간 부족
- 네트워크 백업 경로 연결 실패
- 설정 파일 손상

## 로깅 전략

### 1. 로그 레벨

- **ERROR**: 치명적인 오류 (백업 실패, 설정 오류)
- **WARN**: 경고 (일부 파일 백업 실패)
- **INFO**: 일반 정보 (백업 시작/완료, 설정 변경)
- **DEBUG**: 디버그 (개별 파일 처리, 이벤트 감지)

### 2. 로그 포맷

```rust
// 예시
[2026-02-21 10:00:00.123 INFO ardiex::backup] Starting backup for source: "/data/documents"
[2026-02-21 10:00:05.456 WARN ardiex::backup] Failed to backup file: "/data/documents/locked.tmp"
[2026-02-21 10:00:10.789 ERROR ardiex::backup] Backup failed: Insufficient disk space
[2026-02-21 10:00:15.000 INFO ardiex::commands::run_cmd] [HOT-RELOAD] Applied successfully ...
```

## 성능 최적화

### 1. 병렬 처리

- 여러 소스 동시 백업 (tokio::join!)
- 대용량 파일 스트리밍 처리

### 2. 메모리 관리

- 큰 파일은 chunk 단위로 읽기
- 해시 계산 시 스트림 사용

### 3. 디바운싱

- 파일 변경 이벤트 300ms 딜레이
- 중복 백업 요청 방지

## CLI 디자인 패턴

### 1. clap 사용

```rust
#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    Backup,
    Restore { backup_dir, target_dir, point, list },
    Run,
}

enum ConfigAction {
    Init, List, AddSource, RemoveSource,
    AddBackup, RemoveBackup,
    Set { key, value },           // 글로벌 설정 (cron_schedule, enable_min_interval_by_size, max_log_file_size_mb 포함)
    SetSource { source, key, value },  // 소스별 설정 (cron_schedule 포함)
}

// 모든 경로 입력은 절대경로 필수 (ensure_absolute() 검증)
// 프로그램 시작 시 validate_all_sources()로 전체 설정 검증:
//   글로벌 값, 경로 절대/존재/디렉토리, 중복, 소스==백업 금지,
//   소스별 오버라이드 값, cron 표현식, delta chain, auto full interval
// full_backup_interval은 수동 설정하지 않고 max_backups 기준 자동 계산
```

### 3. 설정 파일 웹 편집기

- 위치: `src/editor/settings-editor.html`
- 단일 HTML 파일 (외부 의존성 없음)
- `settings.json` 파일명 강제 + 로드 시 스키마 검증
- 파일 열기 + 화면 전체 Drag & Drop 지원
- 열기 전 저장 버튼 비활성화, 열린 파일에 덮어쓰기 저장(FS API 지원 시)
- 설정 변경 시 JSON 미리보기 자동 갱신 + 미리보기 JSON 복사 버튼
- 릴리스 아카이브에 포함

### 4. 컬러 출력

- **녹색**: 성공
- **노란색**: 경고
- **빨간색**: 오류
- **파란색**: 정보

## 테스트 전략

- 테스트 코드 위치: `src/tests/*.rs` (모듈에서 `#[path = "..."]`로 연결)
- TDD 케이스 기준 문서: `docs/test-cases/tdd-test-plan.md`
- 기능 추가/코드 수정 시 관련 테스트 코드를 반드시 수정 또는 추가하고, 테스트 실행으로 검증
- 테스트 작성 우선순위: 실패 경로(설정 오류/입력 오류/데이터 손상/경로 오류) -> 정상 경로

### 1. 단위 테스트

- 해시 계산 함수
- 설정 직렬화/역직렬화
- 파일 패턴 매칭
- 런타임/워처 경로 테스트: `src/tests/run_cmd_tests.rs`
- 로그 tee writer 테스트: `src/tests/logger_tests.rs`
- 설정/해상도 테스트: `src/tests/config_tests.rs`
- delta 알고리즘 테스트: `src/tests/delta_tests.rs`
- watcher 이벤트 필터/디바운스 테스트: `src/tests/watcher_tests.rs`

### 2. 통합 테스트

- 전체 백업 흐름
- CLI 명령어 실행
- 설정 파일 조작
- 백업 시나리오 테스트: `src/tests/backup_tests.rs`
- 복구 시나리오 테스트: `src/tests/restore_tests.rs`

### 3. 테스트 유틸리티

- 임시 디렉토리 생성
- 모의 파일 시스템
- 가짜 파일 이벤트 생성
