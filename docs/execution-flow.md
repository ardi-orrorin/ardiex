# Ardiex 실행 로직 전체 다이어그램

이 문서는 `ardiex`의 실행 경로를 한 번에 확인할 수 있도록 mermaid 다이어그램으로 정리한 문서입니다.

## 1) 메인 실행 흐름 (`ardiex`)

```mermaid
flowchart TD
    A[프로세스 시작: ardiex] --> B[settings.json 로드 또는 생성]
    B --> C[로거 초기화: logs/ardiex.log]
    C --> D{업데이트 체크 스킵 조건?}

    D -->|예: ARDIEX_SKIP_UPDATE_CHECK=1 / --help / --version| E[CLI 파싱]
    D -->|아니오| F[GitHub latest release 조회]

    F --> G{최신 버전 존재?}
    G -->|아니오| E
    G -->|예| H[현재 OS/ARCH 에셋 선택]
    H --> I{updater 바이너리 존재?}
    I -->|아니오| E
    I -->|예| J[updater 프로세스 실행 + 인자 전달]
    J --> K[현재 ardiex 프로세스 종료]

    E --> L{명령어 분기}
    L -->|config| M[설정 관리 커맨드 실행]
    L -->|backup| N[수동 백업 실행]
    L -->|restore| O[복구 실행]
    L -->|run| P[서비스 루프 실행]
```

## 2) 업데이트 위임/교체 흐름 (`updater`)

```mermaid
flowchart TD
    A[updater 시작] --> B[로거 초기화: logs/updater.log]
    B --> C[부모 PID 종료 대기]
    C --> D[릴리즈 에셋 다운로드]
    D --> E{압축 형식}
    E -->|.tar.gz| F[tar.gz 압축 해제]
    E -->|.zip| G[zip 압축 해제]
    F --> H[새 ardiex 바이너리 탐색]
    G --> H
    H --> I[대상 실행파일 교체 재시도]
    I --> J[원래 인자로 ardiex 재실행]
    J --> K[임시 파일 정리]
    K --> L[업데이트 완료 로그]
```

## 3) 백업 실행 흐름 (`backup` / `run` 공통 백업 엔진)

```mermaid
flowchart TD
    A[validate_all_sources] --> B[소스/백업경로/cron/metadata 검증]
    B --> C[force_full 여부 계산]
    C --> D[backup_all_sources]
    D --> E[소스별 반복]
    E --> F[백업 경로별 반복]
    F --> G{이번 백업 타입 결정}

    G -->|force_full=true 또는 full 필요| H[full_타임스탬프 디렉토리 생성]
    G -->|incremental 가능| I[inc_타임스탬프 디렉토리 생성]

    H --> J[변경 파일 스캔 + 복사]
    I --> K{backup_mode}
    K -->|delta| L[이전 파일 비교 후 .delta 생성]
    K -->|copy| M[변경 파일 전체 복사]

    J --> N[metadata 업데이트]
    L --> N
    M --> N
    N --> O[inc_checksum 기록/검증 정보 동기화]
    O --> P[보관 정책(max_backups) 정리]
    P --> Q[완료 로그 + 결과 반환]
```

## 4) 서비스 루프 흐름 (`run`)

```mermaid
flowchart TD
    A[run 시작] --> B[초기 설정 로드 + validate_all_sources]
    B --> C[[CONFIG] pretty JSON 출력]
    C --> D[cron task / watcher task 생성]
    D --> E{tokio select 루프}

    E -->|backup trigger 수신| F[backup_all_sources 실행]
    F --> E

    E -->|2초 reload tick| G[settings.json 재로드]
    G --> H{fingerprint 변경?}
    H -->|아니오| E
    H -->|예| I[신규 설정 검증]
    I --> J{유효성}
    J -->|유효| K[기존 task 중단 후 신규 task 교체]
    K --> L[[HOT-RELOAD] Applied + [CONFIG] 출력]
    L --> E
    J -->|무효| M[[HOT-RELOAD] Rejected 로그]
    M --> E

    E -->|Ctrl+C| N[task 정리 후 종료]
```

## 5) 실행 모드 관계 요약

```mermaid
stateDiagram-v2
    [*] --> Full: 첫 실행 / 강제 full
    Full --> Incremental: 다음 주기
    Incremental --> Incremental: 변경분 백업 지속
    Incremental --> Full: max_backups 기반 자동 주기 도달
    Incremental --> Full: delta chain/inc checksum 검증 실패
```
