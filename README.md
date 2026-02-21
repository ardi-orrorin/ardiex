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
  - **copy**: 변경 파일 전체 복사 (주기적 백업만 지원)
- **주기적 full 강제**: 설정된 inc 횟수마다 자동 full 백업
- **Delta 체인 검증**: 백업 시작 시 기존 delta 파일 무결성 검증, 손상 시 full 전환
- **글로벌/소스별 설정**: 소스별 설정이 글로벌 설정을 오버라이드

### 2. 트리거 방식

- **주기적 백업**: 설정된 시간 간격으로 자동 실행
- **I/O 이벤트 기반**: 파일 시스템 변경 감지 시 즉시 실행

## 설정 파일 (settings.json)

### 위치

- 실행 파일과 동일한 경로
- 없으면 기본값으로 자동 생성

### 구조

```json
{
  "sources": [
    {
      "source_dir": "./documents",
      "backup_dirs": ["./documents/.backup", "/backup/documents"],
      "enabled": true,
      "exclude_patterns": ["*.cache"],
      "max_backups": 5,
      "backup_mode": "copy",
      "full_backup_interval": 3
    },
    {
      "source_dir": "./photos",
      "backup_dirs": ["/backup/photos"],
      "enabled": true
    }
  ],
  "periodic_interval_minutes": 60,
  "enable_periodic": true,
  "enable_event_driven": true,
  "exclude_patterns": ["*.tmp", "*.log", ".git/*"],
  "max_backups": 10,
  "backup_mode": "delta",
  "full_backup_interval": 10,
  "metadata": {
    "./documents": {
      "last_full_backup": "2024-02-21T10:00:00Z",
      "last_backup": "2024-02-21T11:30:00Z",
      "file_hashes": {
        "file1.txt": "sha256_hash...",
        "subdir/file2.pdf": "sha256_hash..."
      }
    }
  }
}
```

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

# 주기적 백업 (기본 60분)과 파일 변경 감지 백업이 동시에 실행됨
```

### 5. 설정 변경

```bash
# 글로벌 설정
./ardiex config set periodic_interval_minutes 30
./ardiex config set enable_periodic false
./ardiex config set enable_event_driven false
./ardiex config set max_backups 20
./ardiex config set backup_mode delta          # delta 또는 copy
./ardiex config set full_backup_interval 10    # N번 inc 후 full 강제

# 소스별 설정 (글로벌 오버라이드)
./ardiex config set-source /home/user/documents backup_mode copy
./ardiex config set-source /home/user/documents max_backups 5
./ardiex config set-source /home/user/documents full_backup_interval 3
./ardiex config set-source /home/user/documents exclude_patterns "*.cache,*.tmp"

# 소스별 설정 초기화 (글로벌로 폴백)
./ardiex config set-source /home/user/documents backup_mode reset
```

### 설정 우선순위

소스별 설정이 존재하면 글로벌 설정보다 우선 적용됩니다.

| 설정 키                | 글로벌           | 소스별 (Optional)  |
| ---------------------- | ---------------- | ------------------ |
| `exclude_patterns`     | `["*.tmp", ...]` | 지정 시 오버라이드 |
| `max_backups`          | `10`             | 지정 시 오버라이드 |
| `backup_mode`          | `"delta"`        | 지정 시 오버라이드 |
| `full_backup_interval` | `10`             | 지정 시 오버라이드 |

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
4. **Full 강제 확인**: `full_backup_interval` 도달 시 full 백업 강제
5. **Delta 백업**: 변경된 파일을 4KB 블록 단위로 비교하여 변경된 블록만 저장
6. **자동 판단**: delta 크기가 원본의 50% 미만이면 delta 저장, 아니면 전체 복사
7. **메타데이터 업데이트**: 파일 해시 정보 저장

### Copy 모드 프로세스

1. **파일 해시 계산**: SHA-256으로 각 파일의 해시 계산
2. **변경 감지**: 이전 해시와 비교하여 변경된 파일 식별
3. **파일 복사**: 변경된 파일 전체를 백업 디렉토리에 복사
4. **메타데이터 업데이트**: 파일 해시 정보 저장

> **참고**: copy 모드에서는 실시간(이벤트 기반) 백업이 비활성화되며, 주기적 백업만 지원됩니다.

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
[INFO] Backup progress: 10% (10/100 files)
[INFO] Backup progress: 20% (20/100 files)
...
[INFO] Restore progress: 50% - Applied backup 'full_20240221_100000123': 50 files restored
```

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

1. **config.rs** - 설정 파일 관리
2. **backup.rs** - 증분 백업 로직
3. **delta.rs** - 블록 단위 delta 백업/복원
4. **restore.rs** - 백업 복구 관리
5. **watcher.rs** - 파일 시스템 감시
6. **logger.rs** - 파일 로깅
7. **main.rs** - CLI 인터페이스 및 메인 로직
