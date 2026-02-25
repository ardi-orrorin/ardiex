# Ardiex TDD 테스트 케이스 상세 계획

## 목적

- 테스트를 구현 전에 먼저 정의하여 회귀를 줄이고 설계 품질을 높인다.
- `src/tests`의 테스트 코드를 기능 요구사항과 1:1로 연결한다.
- PR 단위로 `Red -> Green -> Refactor`를 반복 가능한 작업 단위로 만든다.

## 범위

- 단위/모듈 테스트:
  - `src/tests/backup_tests.rs`
  - `src/tests/run_cmd_tests.rs`
  - `src/tests/logger_tests.rs`
  - `src/tests/config_tests.rs`
  - `src/tests/delta_tests.rs`
  - `src/tests/restore_tests.rs`
  - `src/tests/watcher_tests.rs`
  - `src/tests/update_tests.rs`
- 통합 흐름 테스트: `backup`, `run` 핵심 시나리오 (임시 디렉토리 기반)

## TDD 공통 규칙

1. Red: 먼저 실패하는 테스트를 작성한다.
2. Green: 테스트를 통과시키는 최소 코드만 구현한다.
3. Refactor: 중복 제거/구조 개선 후 테스트 전체 재실행으로 동작 보존을 확인한다.
4. 각 테스트는 하나의 행동만 검증한다.
5. 파일 시스템 테스트는 반드시 임시 디렉토리에서 독립 실행한다.
6. 타임스탬프 충돌 가능성이 있으면 최소 sleep(예: 5ms)을 명시한다.
7. 테스트 우선순위는 실패 경로(잘못된 설정/입력/손상 데이터/경로 오류) -> 정상 경로 순으로 둔다.

## 우선순위 기준

- `P0`: 데이터 손상/백업 실패를 막는 필수 시나리오
- `P1`: 런타임 안정성/운영 가시성 시나리오
- `P2`: 경계 조건/리팩토링 안전망 강화 시나리오

## 테스트 케이스 목록 (TDD)

### A. 백업 오케스트레이션 (`src/tests/backup_tests.rs`)

#### TC-BACKUP-001 (P0) 첫 실행은 full 백업이어야 한다

- 대상: `BackupManager::backup_all_sources()`
- Red:
1. 신규 소스/신규 백업 디렉토리 구성 후 첫 백업 결과를 `Incremental`로 기대하도록 일부러 작성해 실패 확인
- Green:
1. 첫 결과가 `BackupType::Full`인지 검증
2. 백업 디렉토리에 `full_` prefix가 생성되는지 검증
- Refactor:
1. 테스트 fixture 생성 코드를 helper로 추출

#### TC-BACKUP-002 (P0) 같은 프로세스 두 번째 실행은 incremental 이어야 한다

- 대상: force-full 플래그 해제 로직
- Red:
1. 첫 백업 후 파일 변경
2. 두 번째도 `Full`을 기대하도록 작성해 실패 확인
- Green:
1. 두 번째 결과가 `BackupType::Incremental`인지 검증
2. `inc_` prefix 디렉토리 생성 확인
- Refactor:
1. 반복되는 파일 변경 유틸 함수를 공통화

#### TC-BACKUP-003 (P0) copy 모드에서 .delta 파일이 생성되면 안 된다

- 대상: copy 모드 분기
- Red:
1. copy 모드 백업 후 `.delta` 존재를 기대하도록 작성해 실패 확인
- Green:
1. incremental 디렉토리에 변경 파일 원본이 복사되었는지 검증
2. 전체 백업 경로에 `.delta`가 없는지 검증
- Refactor:
1. 디렉토리 재귀 검사 로직 helper 유지

#### TC-BACKUP-004 (P0) 증분 체크섬이 metadata에 기록되어야 한다

- 대상: metadata 기록
- Red:
1. `inc_checksum`이 `None`이어야 한다고 작성해 실패 확인
- Green:
1. 첫 full 이후 inc 생성
2. `metadata.json`의 inc 이력에서 `inc_checksum.is_some()` 검증
- Refactor:
1. metadata 로드/파싱 helper 함수 분리

#### TC-BACKUP-005 (P0) inc 데이터 변조 시 다음 백업은 full 강제되어야 한다

- 대상: 시작 시 `inc_checksum` 검증
- Red:
1. inc 변조 후 다음 백업 타입을 `Incremental`로 기대하도록 작성해 실패 확인
- Green:
1. 변조 후 재시작(manager 재생성)
2. 다음 백업 결과가 `Full`인지 검증
- Refactor:
1. 변조 대상 파일 찾기 로직 함수화

#### TC-BACKUP-006 (P0) `max_backups` 기반 자동 full 주기가 적용되어야 한다

- 대상: 자동 full 주기 계산(`max_backups - 1`, 최소 1)
- Red:
1. `max_backups=3`에서 `full -> inc -> inc -> inc` 기대로 작성해 실패 확인
- Green:
1. 실제 기대는 `full -> inc -> inc -> full`
- Refactor:
1. 단계별 백업 결과 수집 helper 추가

#### TC-BACKUP-007 (P1) 다중 backup_dir 모두에 결과가 생성되어야 한다

- 대상: 다중 백업 경로 처리
- Red:
1. 두 경로 중 하나만 생성 기대로 실패 확인
- Green:
1. 두 경로 모두 full/inc 디렉토리 생성 검증
- Refactor:
1. 경로별 검증 루프 공통화

#### TC-BACKUP-008 (P1) 비활성화된 소스는 백업 대상에서 제외되어야 한다

- 대상: source `enabled=false` 필터링
- Red:
1. 비활성 소스 결과가 1개 이상이어야 한다고 작성해 실패 확인
- Green:
1. 결과 목록에서 해당 소스 결과가 0개인지 검증
- Refactor:
1. 다중 소스 fixture builder 도입

#### TC-BACKUP-009 (P1) exclude pattern 대상 파일은 백업되지 않아야 한다

- 대상: 제외 패턴 적용
- Red:
1. `*.tmp` 파일이 백업되어야 한다고 작성해 실패 확인
- Green:
1. `a.tmp` 제외, `a.txt` 포함 검증
- Refactor:
1. 테스트 파일 생성 헬퍼로 중복 제거

#### TC-BACKUP-010 (P1) metadata 이력과 디스크 불일치 시 안전 동작해야 한다

- 대상: 시작 시 metadata-history 검증
- Red:
1. 이력 불일치에서도 normal incremental 기대로 실패 확인
- Green:
1. 정책에 맞는 동작(검증 에러 또는 full 강제)을 명시적으로 assert
- Refactor:
1. 정책별 assertion 유틸 분리

#### TC-BACKUP-011 (P2) 삭제된 파일은 후속 metadata에서 제거되어야 한다

- 대상: 파일 해시 동기화
- Red:
1. 삭제 파일 해시가 유지되어야 한다고 작성해 실패 확인
- Green:
1. 삭제 후 백업 시 metadata `file_hashes`에서 제거 검증
- Refactor:
1. metadata key 조회 helper 추가

#### TC-BACKUP-012 (P2) delta 모드에서 이전 파일이 있으면 delta 파일 생성되어야 한다

- 대상: delta 생성 조건
- Red:
1. delta 모드에서 `.delta`가 없어야 한다고 작성해 실패 확인
- Green:
1. 조건 충족 시 `.delta` 생성 검증
- Refactor:
1. delta 존재 확인 함수 재사용

### B. run/hot-reload (`src/tests/run_cmd_tests.rs`)

#### TC-RUN-001 (P0) copy 모드 + 이벤트 활성 시 watch 경로가 수집되어야 한다

- 대상: `collect_event_watch_paths()`
- Red:
1. 경로가 비어 있어야 한다고 작성해 실패 확인
- Green:
1. source 경로 1개 포함 검증
- Refactor:
1. 공통 config builder 유지

#### TC-RUN-002 (P0) 글로벌 이벤트 비활성 시 watch 경로는 비어야 한다

- 대상: 글로벌 스위치
- Red:
1. 경로가 존재해야 한다고 작성해 실패 확인
- Green:
1. 빈 벡터 검증
- Refactor:
1. assertion helper 도입

#### TC-RUN-003 (P0) 소스별 이벤트 비활성 override가 글로벌보다 우선해야 한다

- 대상: source override precedence
- Red:
1. 경로 포함 기대로 실패 확인
- Green:
1. 경로 제외 검증
- Refactor:
1. override 시나리오 테이블 테스트 전환

#### TC-RUN-004 (P1) disabled source는 watch 대상에서 제외되어야 한다

- 대상: `enabled` 필터
- Red:
1. disabled source 포함 기대로 실패 확인
- Green:
1. 제외 검증
- Refactor:
1. source 생성 helper 확장

#### TC-RUN-005 (P1) config snapshot은 pretty JSON 포맷이어야 한다

- 대상: `config_snapshot_pretty_json()`
- Red:
1. compact JSON만 허용하도록 작성해 실패 확인
- Green:
1. 줄바꿈/들여쓰기 포함 여부, `phase`, `config` 키 존재 검증
- Refactor:
1. snapshot parsing helper 추가

#### TC-RUN-006 (P1) 동일한 invalid fingerprint는 반복 로그 스팸을 방지해야 한다

- 대상: hot-reload 실패 fingerprint 캐시
- Red:
1. 동일 invalid 설정에서 매 tick마다 재시도 기대로 실패 확인
- Green:
1. 첫 실패 이후 동일 fingerprint는 skip됨을 검증
- Refactor:
1. reload loop를 테스트 가능한 함수 단위로 분리

### C. 로거 (`src/tests/logger_tests.rs`)

#### TC-LOGGER-001 (P0) write는 파일/콘솔에 동일 payload를 기록해야 한다

- 대상: `TeeLogWriter::write()`
- Red:
1. 서로 다른 payload 기대로 실패 확인
- Green:
1. 동일 버퍼/쓰기 횟수 검증
- Refactor:
1. mock writer 재사용

#### TC-LOGGER-002 (P0) 파일 write 실패 시 stdout write는 실행되지 않아야 한다

- 대상: 실패 전파 순서
- Red:
1. stdout write가 1회 이상이라고 기대해 실패 확인
- Green:
1. 에러 메시지 + stdout writes=0 검증
- Refactor:
1. 실패 mock builder 도입

#### TC-LOGGER-003 (P0) stdout write 실패는 파일 write 이후 에러로 반환되어야 한다

- 대상: 두 번째 write 실패 경로
- Red:
1. 파일 write도 실패한다고 기대해 실패 확인
- Green:
1. 파일 writes=1, stdout 에러 반환 검증
- Refactor:
1. 공통 assert 헬퍼 추가

#### TC-LOGGER-004 (P1) flush는 두 타겟 모두 호출되어야 한다

- 대상: `TeeLogWriter::flush()`
- Red:
1. flush count=0 기대로 실패 확인
- Green:
1. file/stdout flush count=1 검증
- Refactor:
1. flush 관련 helper 분리

#### TC-LOGGER-005 (P1) 파일 flush 실패 시 stdout flush는 실행되지 않아야 한다

- 대상: flush 에러 경로
- Red:
1. stdout flush도 수행 기대로 실패 확인
- Green:
1. 에러 반환 + stdout flush count=0 검증
- Refactor:
1. 실패 path 테스트 데이터화

#### TC-LOGGER-006 (P2) stdout flush 실패는 파일 flush 이후 에러로 반환되어야 한다

- 대상: flush 두 번째 타겟 실패 경로
- Red:
1. 파일 flush 미실행 기대로 실패 확인
- Green:
1. 파일 flush 실행됨 + stdout 에러 검증
- Refactor:
1. mock 시나리오 파라미터화

### D. 업데이트 버전/에셋 선택 (`src/tests/update_tests.rs`)

#### TC-UPDATE-001 (P0) 버전 문자열 정규화는 접두사/메타정보를 제거해야 한다

- 대상: `normalize_version()`
- Red:
1. `v1.2.3-beta+meta`가 그대로 남아야 한다고 작성해 실패 확인
- Green:
1. `v`/`V`, prerelease, build metadata 제거 결과 검증
- Refactor:
1. 입력 케이스를 테이블화

#### TC-UPDATE-002 (P0) semver 비교는 숫자 크기대로 정렬되어야 한다

- 대상: `compare_versions()`, `is_newer_version()`
- Red:
1. `1.10.0 < 1.2.0` 기대로 작성해 실패 확인
- Green:
1. `Ordering` 결과와 최신 버전 판별 결과 검증
- Refactor:
1. 비교 helper로 중복 제거

#### TC-UPDATE-003 (P1) 릴리즈 에셋 누락 시 명확한 오류를 반환해야 한다

- 대상: `find_release_asset_download_url()`
- Red:
1. 누락 에셋에서도 URL이 반환된다고 기대해 실패 확인
- Green:
1. 에러 메시지에 누락된 에셋 정보 포함 검증
- Refactor:
1. 가짜 릴리즈 fixture 재사용

#### TC-UPDATE-004 (P1) 플랫폼 에셋 매핑은 지원 타깃만 허용해야 한다

- 대상: `expected_release_asset_name_for_current_target()`
- Red:
1. 현재 타깃에서 빈 문자열 반환을 기대해 실패 확인
- Green:
1. 현재 OS/ARCH 조합에서 유효한 에셋명 포맷 반환 검증
- Refactor:
1. 포맷 검증 공통 helper화

## 케이스-코드 매핑 (자동화 현황)

| 케이스 | 상태 | 테스트 함수 |
| --- | --- | --- |
| TC-BACKUP-001 | 자동화 완료 | `clears_force_full_after_first_full_in_same_process` |
| TC-BACKUP-002 | 자동화 완료 | `clears_force_full_after_first_full_in_same_process` |
| TC-BACKUP-003 | 자동화 완료 | `copy_mode_creates_incremental_copy_without_delta_file` |
| TC-BACKUP-004 | 자동화 완료 | `validates_incremental_checksum_from_metadata_on_startup` |
| TC-BACKUP-005 | 자동화 완료 | `validates_incremental_checksum_from_metadata_on_startup` |
| TC-BACKUP-006 | 자동화 완료 | `max_backups_interval_forces_full_after_restart_in_delta_mode` |
| TC-BACKUP-007 | 자동화 완료 | `multi_backup_dirs_receive_full_and_incremental_backups` |
| TC-BACKUP-008 | 자동화 완료 | `disabled_source_is_skipped_in_backup_all_sources` |
| TC-BACKUP-009 | 자동화 완료 | `full_backup_respects_exclude_patterns` |
| TC-BACKUP-010 | 자동화 완료 | `validate_all_sources_forces_full_on_metadata_history_mismatch` |
| TC-BACKUP-011 | 자동화 완료 | `deleted_file_is_removed_from_metadata_hashes` |
| TC-BACKUP-012 | 자동화 완료 | `delta_mode_creates_delta_file_when_previous_backup_exists` |
| TC-RUN-001 | 자동화 완료 | `collect_event_watch_paths_includes_copy_mode_source` |
| TC-RUN-002 | 자동화 완료 | `collect_event_watch_paths_empty_when_global_disabled` |
| TC-RUN-003 | 자동화 완료 | `collect_event_watch_paths_respects_source_override_disable` |
| TC-RUN-004 | 자동화 완료 | `collect_event_watch_paths_skips_disabled_sources` |
| TC-RUN-005 | 자동화 완료 | `config_snapshot_pretty_json_contains_phase_and_config` |
| TC-RUN-006 | 자동화 완료 | `should_skip_hot_reload_when_latest_matches_failed_fingerprint` |
| TC-LOGGER-001 | 자동화 완료 | `tee_writer_writes_same_payload_to_both_targets` |
| TC-LOGGER-002 | 자동화 완료 | `tee_writer_stops_when_file_write_fails` |
| TC-LOGGER-003 | 자동화 완료 | `tee_writer_returns_stdout_error_after_file_write` |
| TC-LOGGER-004 | 자동화 완료 | `tee_writer_flushes_both_targets` |
| TC-LOGGER-005 | 자동화 완료 | `tee_writer_stops_when_file_flush_fails` |
| TC-LOGGER-006 | 자동화 완료 | `tee_writer_returns_stdout_flush_error_after_file_flush` |
| TC-UPDATE-001 | 자동화 완료 | `normalize_version_strips_prefix_and_prerelease` |
| TC-UPDATE-002 | 자동화 완료 | `compare_versions_orders_semver_triplets`, `is_newer_version_detects_candidate_newer` |
| TC-UPDATE-003 | 자동화 완료 | `find_release_asset_download_url_returns_error_for_missing_asset` |
| TC-UPDATE-004 | 부분 자동화 | `expected_release_asset_name_for_current_target` 직접 테스트 추가 예정 |

## 추가 회귀 테스트

- 설정 병합/기본값/자동 full 주기: `src/tests/config_tests.rs`
- delta 생성/적용/직렬화 roundtrip: `src/tests/delta_tests.rs`
- 복구 선택/적용/cutoff/empty 처리: `src/tests/restore_tests.rs`
- watcher 필터/디바운스/버스트 이벤트 처리: `src/tests/watcher_tests.rs`
- 업데이트 버전 비교/에셋 선택 오류: `src/tests/update_tests.rs`

## 실패 경로 커버리지 인덱스

- 설정/검증 실패:
  - `src/tests/backup_tests.rs`: 중복 source/backup, 상대경로, source==backup, source 미존재/파일경로, source/global cron 오류, source/global max_backups=0, max_log_file_size_mb=0
  - `src/tests/config_tests.rs`: invalid `backup_mode`, 필수 필드 누락 역직렬화 실패
- 데이터 손상/포맷 실패:
  - `src/tests/backup_tests.rs`: metadata history 불일치, full 없는 inc history
  - `src/tests/delta_tests.rs`: invalid delta JSON, missing file/new file
  - `src/tests/restore_tests.rs`: invalid delta 복구 실패
  - `src/tests/update_tests.rs`: 릴리즈 에셋 누락 오류
- 런타임/핫리로드 실패:
  - `src/tests/run_cmd_tests.rs`: invalid cron 거부, hot-reload skip 조건 검증
  - `src/tests/watcher_tests.rs`: temp/lock 이벤트 무시, sender disconnect 시 트리거 없음

## 구현 순서 권장 (TDD 스프린트)

1. Sprint 1 (P0): TC-BACKUP-001~006, TC-RUN-001~003, TC-LOGGER-001~003, TC-UPDATE-001~002
2. Sprint 2 (P1): TC-BACKUP-007~010, TC-RUN-004~006, TC-LOGGER-004~005, TC-UPDATE-003~004
3. Sprint 3 (P2): TC-BACKUP-011~012, TC-LOGGER-006

## PR 체크리스트

1. 새 기능마다 Red 커밋(실패 테스트) 이력이 있는가
2. Green 커밋에서 최소 구현으로 테스트를 통과시켰는가
3. Refactor 커밋에서 테스트가 동일하게 통과하는가
4. `cargo test --all-targets -q` 전체 통과 여부
5. 새 테스트가 `src/tests` 폴더 규칙을 따르는가
6. 기능 추가/코드 수정 시 관련 테스트를 수정 또는 추가했는가

## 실행 명령 예시

```bash
cargo test --all-targets -q
cargo test backup_tests -q
cargo test run_cmd_tests -q
cargo test logger_tests -q
cargo test config_tests -q
cargo test delta_tests -q
cargo test restore_tests -q
cargo test watcher_tests -q
cargo test update_tests -q
```
