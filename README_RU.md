# lightning

[In English](README.md)

lightning — self-hosted open-source платформа наблюдаемости и ускорения Gradle CI (в первую очередь для Android-монорепо): слой **над** сборкой, никогда не внутри неё. Сейчас платформа включает четыре возможности: **радар flaky-тестов** (CLI загружает JUnit XML из CI; сервер ведёт историю тестов, считает детерминированный flaky score и показывает, что флапает, с какого момента и на каком коммите), **телеметрию сборок** («build scans lite»: Gradle init-скрипт отправляет на тот же сервер тайминги тасок, результаты кэша, время конфигурации и всей сборки), **remote build cache** с аналитикой (сервер реализует HTTP-протокол кэша Gradle и показывает hit rate, статистику хранилища и некэшируемые таски поверх телеметрии) и **селективное выполнение** (`lightning sync`/`affected`/`run`: один раз снимаем граф модулей, дальше решаем, что затронул diff, на чистом Rust — до старта любой JVM).

## Запуск сервера

```sh
cargo build --release
./target/release/lightning-server --addr 0.0.0.0:8080 --db lightning.db --retention-days 90
```

Флаги доступны и как переменные окружения: `LIGHTNING_ADDR`, `LIGHTNING_DB`, `LIGHTNING_RETENTION_DAYS`. UI: `/` (список flaky), `/tests/{id}` (история теста), `/runs/{id}` (сводка прогона), `/builds` (список сборок), `/builds/{id}` (детали сборки), `/trends` (тренды по веткам), `/cache` (хранилище и аналитика кэша); JSON — `/api/flaky` и `/api/builds`.

## Загрузка результатов тестов из CI

Один шаг после тестов (интеграция в билд не нужна):

```yaml
- name: Upload test results to lightning
  if: always()
  run: lightning upload --server https://lightning.example.com
```

`lightning upload` парсит отчёты по маске `**/build/test-results/**/*.xml` (переопределяется `--glob`), берёт SHA/ветку и идентичность прогона из окружения GitHub Actions или локального git-репозитория и загружает идемпотентно — повторный запуск шага никогда не создаёт дубликат прогона.

## Телеметрия сборок в CI

Извлеките вшитый init-скрипт и подключите его к сборке:

```yaml
- name: Enable lightning build telemetry
  run: lightning init-script --out "$RUNNER_TEMP/lightning.init.gradle"

- name: Build
  run: ./gradlew build --init-script "$RUNNER_TEMP/lightning.init.gradle"
  env:
    LIGHTNING_URL: https://lightning.example.com
```

Извлекайте скрипт вне репозитория (как выше): `.gradle`-файл внутри рабочего дерева попадает в инвалидационный набор селективного выполнения и потребует лишний `lightning sync`.

Скрипт собирает тайминги и результаты тасок (success / up-to-date / from-cache / failed / skipped), время конфигурации и всей сборки, запрошенные таски, версии Gradle/JDK и git/CI-метаданные, а по завершении сборки отправляет один JSON-документ в `/api/builds`. URL сервера задаётся Gradle-свойством `lightning.url` (`-Plightning.url=...`) или переменной `LIGHTNING_URL`. Телеметрия fail-safe: без URL ничего не делает, любая ошибка логируется и глотается — сборку она не ломает никогда.

## Remote build cache

Сервер реализует HTTP-протокол кэша Gradle на `/cache/{key}`. Включите его в `settings.gradle(.kts)` — push из CI, pull отовсюду:

```groovy
// settings.gradle
buildCache {
    remote(HttpBuildCache) {
        url = 'https://lightning.example.com/cache/'   // слэш в конце обязателен
        push = System.getenv('CI') != null
        credentials {
            username = 'ci'                            // сервер игнорирует
            password = System.getenv('LIGHTNING_CACHE_TOKEN') ?: ''
        }
    }
}
```

Запускайте сборки с `--build-cache` (или `org.gradle.caching=true`). Хранилище ограничено и обслуживает себя само: артефакты лежат в каталоге рядом с БД (`--cache-dir` / `LIGHTNING_CACHE_DIR`), артефакты крупнее 100 MiB отклоняются (`--cache-max-artifact-mb`), общий объём ограничен 10 GiB с LRU-выселением (`--cache-max-size-mb`), записи без обращений 30 дней удаляются (`--cache-retention-days`).

Запись можно защитить общим токеном: запустите сервер с `LIGHTNING_CACHE_TOKEN=<secret>` (или `--cache-token`) и передайте то же значение в CI — Gradle отправит его как пароль Basic-auth, имя пользователя игнорируется. Чтение всегда открыто; без токена открыта и запись. Аналитика кэша (hit rate, крупнейшие артефакты, never-cached таски) — на `/cache`, и она становится точнее по мере накопления телеметрии.

## Запуск только затронутого diff'ом

Селективному выполнению сервер не нужен. `lightning sync` один раз запускает Gradle со вшитым init-скриптом и пишет `lightning.lock`: модули, их объявленные source-set директории (включая внешние вроде `srcDir("../shared")`), имена тасок и рёбра зависимостей с типами `main`/`test`. Горячий путь не поднимает JVM:

```sh
lightning sync                      # один раз и при изменении билд-файлов
lightning affected                  # затронутые модули, по одному на строку
lightning affected --json           # + причины по модулям и merge-base
lightning run test -- --continue    # gradle :m:test только для затронутых
```

Diff считается как `merge-base(base, HEAD)..HEAD` плюс незакоммиченные изменения (отключается `--no-uncommitted`). База по умолчанию — `origin/main`; переопределяется `--base <ref>`, либо `--base-sha <sha>`, если CI уже знает точный коммит. Shallow clone детектируется с понятной ошибкой (используйте `fetch-depth: 0`).

Модуль затронут, если содержит изменённые файлы, если изменённый модуль достижим из него по `main`-рёбрам (транзитивно) или если одно из его прямых `test`-рёбер (`testImplementation` и родственные) указывает в это множество — `test`-рёбра дальше не распространяются. Всё неоднозначное — файл вне всех модулей, изменение под корнем included build — выбирает **всё**: false negative исключён, лишний прогон допустим. Plugin-only composite builds (`includeBuild("build-logic")` с convention-плагинами) полностью поддерживаются; composite, где зависимость модуля подставляется (dependency substitution) в included build, по-прежнему деградирует до «всё затронуто».

Lock инвалидируется хэшем по всем билд-файлам (`**/*.gradle(.kts)`, `buildSrc/**`, `build-logic/**`, корень каждого included build, записанный в lock, version catalog, wrapper и properties-файлы); устаревший lock завершает работу с кодом 4, если не передан `--auto-sync`. Кэшируйте `lightning.lock` в CI по ключу из этого набора файлов или коммитьте его — работает и так, и так.

Опциональный `lightning.toml` рядом с lock:

```toml
[affected]
base = "origin/main"       # база по умолчанию
ignore = ["docs/**"]       # opt-in: исключить пути из diff (дефолтов нет)
invalidate_on = ["ci/**"]  # дополнительные глобы инвалидации lock
```

`lightning run <task>` сопоставляет имя таски точным совпадением со списком тасок каждого затронутого модуля (Android-модуль запустит `testDebugUnitTest`, а JVM-модуль — `test`), пропускает модули без таски с пометкой и завершается кодом Gradle — либо кодом 0 без запуска Gradle, если ничего не затронуто.

Fast-exit для docs-only PR — раньше, чем checkout успеет поставить JDK (код 3 = ничего не затронуто, 0 = есть затронутое, 4 = устаревший lock):

```yaml
- run: |
    if lightning affected --quiet --auto-sync; then
      echo "run=true" >> "$GITHUB_OUTPUT"
    fi
```

Или fan-out тяжёлых runner'ов только на затронутые модули через `lightning affected --format github-matrix`:

```yaml
jobs:
  plan:
    runs-on: ubuntu-latest
    outputs:
      matrix: ${{ steps.plan.outputs.matrix }}
    steps:
      - uses: actions/checkout@v4
        with: { fetch-depth: 0 }
      - id: plan
        run: echo "matrix=$(lightning affected --format github-matrix --auto-sync)" >> "$GITHUB_OUTPUT"
  test:
    needs: plan
    if: ${{ fromJSON(needs.plan.outputs.matrix).include[0] }}
    strategy:
      matrix: ${{ fromJSON(needs.plan.outputs.matrix) }}
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: ./gradlew ${{ matrix.module }}:test
```

## Лицензия

Apache-2.0. См. [LICENSE](LICENSE).
