# rmsctl

CLI-Client fuer die NeoDEM Roboter-Flottenmanagement-Plattform.

![rmsctl demo](tapes/demo.gif)

## Installation

```bash
cargo build --release
cp target/release/rmsctl ~/.local/bin/
```

## Konfiguration

Ohne Konfiguration verbindet sich `rmsctl` mit `http://localhost:3001/api` (kein Token noetig im Dev-Modus).

```bash
# Per Flag
rmsctl --server http://host:3001/api --token <jwt> status

# Per Umgebungsvariable
export RMS_SERVER_URL=http://host:3001/api
export RMS_TOKEN=<jwt>

# Per Konfigurationsdatei
cat ~/.config/rmsctl/config.json
```

```json
{
  "server_url": "http://localhost:3001/api",
  "token": "optional-jwt"
}
```

Reihenfolge: CLI-Flag > Umgebungsvariable > Konfigurationsdatei > Default.

`~/.config/rmsctl/config.json` wird auf allen Plattformen zuerst gelesen. Zusaetzlich
wird das plattformspezifische Config-Verzeichnis als Fallback geprueft (macOS:
`~/Library/Application Support/rmsctl/config.json`, Windows: `%APPDATA%\rmsctl\config.json`).

## Globale Flags

```
--json           Server-Payload unveraendert als JSON ausgeben
--yes            Zustandsaendernde Befehle wirklich ausfuehren
--page <N>       Seite einer Liste
--limit <N>      Eintraege pro Seite
--server <url>   Server-URL
--token <jwt>    Auth-Token
```

### Der `--yes`-Schalter

Jeder Befehl, der Flotten- oder Deployment-Zustand aendert, beschreibt ohne `--yes`
nur, was er tun wuerde, und bricht ab:

```bash
$ rmsctl safety estop sim-robot-001 --reason "Kollisionsgefahr"

  [!] This will ENGAGE EMERGENCY STOP on sim-robot-001 — reason: Kollisionsgefahr
  [-->] Re-run with --yes to execute.
```

Lesende Befehle sind nie gesperrt.

## Befehle

### Flotte & Sicherheit

```bash
rmsctl status                                    # Flotten-Dashboard (inkl. E-Stop-Status)
rmsctl robots list                               # Alle Roboter auflisten
rmsctl robots show <id>                          # Roboter-Details
rmsctl robots telemetry <id> [--history]         # Telemetrie (CPU, Joints, Sensoren)
rmsctl robots agent-mode <id>                    # Agent-Mode & E-Stop eines Roboters
rmsctl robots cameras <id>                       # Kameras
rmsctl robots peers <id>                         # A2A-Peers
rmsctl robots register <url>                     # Roboter registrieren
rmsctl robots remove <id>                        # Roboter entfernen
rmsctl robots command <id> <type> [--payload {}] # Befehl senden

rmsctl safety status [--robot <id>]              # E-Stop- und Sicherheitszustand
rmsctl safety estop <id> --reason <text>         # Nothalt ausloesen
rmsctl safety estop --fleet --reason <text>      # Flotten-Nothalt
rmsctl safety reset <id> | --fleet               # Nothalt zuruecksetzen
rmsctl safety zone-estop <zone-id> --reason <t>  # Nothalt fuer eine Zone
rmsctl safety events                             # Sicherheits-Ereignisse
```

### Alerts & Incidents

```bash
rmsctl alerts list [--active] [--severity X]     # Alerts
rmsctl alerts show <id>                          # Einzelner Alert
rmsctl alerts ack <id>                           # Alert bestaetigen
rmsctl alerts history                            # Alert-Historie
rmsctl alerts delete <id>                        # Alert loeschen
rmsctl alerts clear --acknowledged | --all       # Alerts massenweise loeschen

rmsctl incidents list [--status X] [--severity X] [--robot-id X]
rmsctl incidents show <id>                       # Incident-Details
rmsctl incidents dashboard                       # Incident-Rollup
rmsctl incidents open | overdue | risk-matrix
rmsctl incidents evidence <id>                   # Angehaengte Nachweise
```

### Aufsicht & Freigaben

```bash
rmsctl oversight dashboard | fleet               # Aufsichts-Uebersicht
rmsctl oversight anomalies [--active] [--unacked]
rmsctl oversight ack-anomaly <id>
rmsctl oversight resolve-anomaly <id> [--notes X]
rmsctl oversight manual-sessions                 # Manuelle Kontrollsitzungen
rmsctl oversight manual-mode <robot-id> [--reason X] [--off]
rmsctl oversight verifications [--due]
rmsctl oversight capabilities <robot-id>
rmsctl oversight logs

rmsctl approvals list | overdue | nearing        # Freigabe-Anfragen
rmsctl approvals pending [--role <r>]            # Was auf dich wartet
rmsctl approvals show <id>
rmsctl approvals decide <id> <step-id> --decision approve|reject|defer|request_info [--notes X]
rmsctl approvals cancel <id> | escalate <id>
rmsctl approvals metrics | sla-report
```

### Teleoperation & Patrouille

```bash
rmsctl teleop list [--robot-id X] [--status X] [--type X]
rmsctl teleop show <id> | episodes <id>
rmsctl teleop pause <id> | resume <id> | end <id>

rmsctl patrol routes | route <id>                # Patrouillen-Routen
rmsctl patrol runs [--route <id>] | run <run-id> # Laeufe
rmsctl patrol findings | finding <id>            # Befunde
rmsctl patrol places                             # Bekannte Orte
rmsctl patrol start <route-id> [--robot-id X] [--mode baseline|patrol]
rmsctl patrol abort <route-id> [--reason X]
rmsctl patrol ack-finding <id> | normal-finding <id> | escalate-finding <id>
rmsctl patrol promote-run <run-id>               # Lauf als Baseline uebernehmen
```

### Daten & Training

```bash
rmsctl datasets list                             # Datensaetze auflisten
rmsctl datasets show <id>                        # Datensatz-Details
rmsctl datasets episodes <id>                    # Episoden eines Datensatzes
rmsctl datasets quality <id> | stats <id>        # Qualitaet & Statistik
rmsctl datasets validate <id> [--advanced]       # Validierung
rmsctl datasets push <id> --hf-token X --repo-id org/name
rmsctl datasets push-status <id>
rmsctl datasets upload <dir> [--name X]          # LeRobot-Verzeichnis direkt hochladen (ohne Hugging Face)
rmsctl datasets upload-status <id>               # Was der Server von einem Upload haelt
rmsctl datasets upload-cancel <id>               # Upload abbrechen, Gespeichertes loeschen
rmsctl datasets flagged <id>                     # Markierte Episoden
rmsctl datasets flag <id> <index> [--reason X]

rmsctl training list                             # Trainingsjobs auflisten
rmsctl training show <id>                        # Job-Details & Fortschritt
rmsctl training active | queue | workers         # Laufend, Warteschlange, Worker
rmsctl training start --config job.json          # Job starten
rmsctl training cancel <id> | retry <id>
rmsctl training export <id> | estimate <id>
```

#### Datensatz direkt hochladen

`rmsctl datasets upload` schickt ein LeRobot-Verzeichnis (v2.x oder v3.0) vom eigenen
Rechner direkt in eine RMS-Instanz — ohne Umweg ueber Hugging Face. Der Datensatz landet
im selben Zustand wie nach einem Hub-Import: Datensatzseite, Episoden-Browser, Video und
Training funktionieren unveraendert.

```bash
# 1. Ansehen, was passieren wuerde (sendet nichts)
rmsctl --server https://neodem.example/api datasets upload /data/g1-dex3-bottle-sorting-sim

# 2. Hochladen, auf die Validierung warten, Ergebnis ausgeben
rmsctl --server https://neodem.example/api datasets upload /data/g1-dex3-bottle-sorting-sim \
  --name "G1 Dex3 bottle sorting" --yes
```

```
  ◈ Upload: G1 Dex3 bottle sorting
    │ Source         /data/g1-dex3-bottle-sorting-sim
    │ Format         LeRobot v3.0 · unitree_g1_dex3 · 30 fps · 80 episodes
    │ Files          7 (446.1 MB) — 1 other entry left out
  [OK] Upload started: dataset 3fb4173c-… (rustfs store, 32 MiB parts, 19 part(s))
    │ Uploading      ████████░░░░░░░░░░░░  41.2%  183.8 MB / 446.1 MB · 5/7 files · 12.4 MB/s · ETA 21s
  [OK] Sent 446.1 MB in 36s (12.4 MB/s)
  [--] The server is assembling the files and verifying checksums …
  [OK] Dataset ready
    │ ID             3fb4173c-81e8-4311-aa13-1b1970f886fc
    │ Status         ● ready
    │ Episodes       80
    │ Frames         55 695
    │ Validation
    │ Result         valid
```

`<dir>` ist das Verzeichnis, das `meta/` und `data/` enthaelt. Gesendet werden nur die
Dateien eines LeRobot-Baums (`meta/**`, `data/**/*.parquet`, `videos/**/*.mp4`,
`images/**`); `README.md`, `.git` usw. bleiben liegen. Ohne `videos/` entsteht ein
Datensatz im Modus `metadata`.

| Flag | Bedeutung |
| --- | --- |
| `--name`, `--description` | Name (Default: Verzeichnisname) und Beschreibung |
| `--robot-type <id>` | Robotertyp festlegen; sonst wird er aus `meta/info.json` zugeordnet oder angelegt |
| `--chunk-size <groesse>` | Teilgroesse, z. B. `8M` (min. 5M; Default und Maximum setzt der Server: 32M) |
| `--parallel <n>` | Gleichzeitig gesendete Teile (Default 3; der Server nimmt insgesamt 4 an) |
| `--retries <n>` | Versuche pro Teil nach voruebergehenden Fehlern, mit Backoff (Default 8) |
| `--resume <id>` | Einen bestimmten Upload fortsetzen (z. B. von einem anderen Rechner) |
| `--restart` | Den gemerkten Upload verwerfen und neu beginnen |
| `--no-checksum` | Kein SHA-256 ueber ganze Dateien (Teile werden weiterhin einzeln geprueft) |
| `--no-wait` | Nach dem letzten Teil zurueckkehren, nicht auf die Validierung warten |
| `--wait-timeout`, `--part-timeout` | Sekunden fuer das Urteil des Servers (1800) bzw. fuer einen Teil (300) |

- **Fortsetzen:** Bricht der Upload ab (Ctrl-C, Netz weg, Rechner aus), denselben Befehl
  noch einmal ausfuehren. `rmsctl` fragt den Server, welche Teile er schon hat, und sendet
  nur den Rest. Lokal wird nur gemerkt, welche Upload-ID zu welchem Verzeichnis gehoert
  (`~/.local/state/rmsctl/uploads/`, ueberschreibbar mit `RMSCTL_STATE_DIR`) — nie im
  Datensatz-Verzeichnis selbst.
- **Abbrechen:** `rmsctl datasets upload-cancel <id> --yes` entfernt alle gespeicherten
  Teile und den Datensatz-Eintrag. Das gilt auch fuer einen hochgeladenen Datensatz, den
  die Validierung abgelehnt hat. Ein Upload ohne Aktivitaet verfaellt serverseitig nach
  6 Stunden von selbst.
- **Speicherbedarf:** Dateien werden von der Platte gestreamt, nie ganz geladen. Der
  Speicherverbrauch haengt weder von Datei- noch von Teilgroesse noch von `--parallel` ab
  (gemessen: ca. 30 MB fuer 446 MB mit zwei 210-MB-Videos).
- **Pruefsummen:** Jeder Teil traegt seinen SHA-256 (`X-Chunk-Sha256`); zusaetzlich wird
  pro Datei ein SHA-256 angemeldet, den der Server nach dem Zusammensetzen gegenliest.
- **Hinter einem Proxy:** Jede Anfrage traegt hoechstens einen Teil und wartet nie auf
  lange Serverarbeit. Auf einer langsamen Leitung `--chunk-size 8M` waehlen: ein Teil muss
  innerhalb des Proxy-Timeouts (ca. 100 s) ankommen.
- **Exit-Code:** 0 nur, wenn der Datensatz `ready` ist. Bei `failed` (Validierung oder
  Pruefsumme) werden die Befunde ausgegeben und der Befehl endet mit 1; Ctrl-C endet mit 130.
- **`--json`:** keine Fortschrittsanzeige; am Ende steht der unveraenderte
  `GET /datasets/<id>`-Payload auf stdout.

### Modelle & Deployment

```bash
rmsctl models list                               # Modell-Versionen
rmsctl models show <id>                          # Version + Evaluierungs-Rollup
rmsctl models lineage <id>                       # Herkunft einer Version
rmsctl models compare <idA> <idB> [--metrics loss,acc]  # Metriken vergleichen

rmsctl deployments list [--active]               # Deployments auflisten
rmsctl deployments show <id> | metrics <id>      # Details & Metriken
rmsctl deployments promote <id>                  # Naechste Stufe
rmsctl deployments rollback <id> [--reason X]    # Zurueckrollen
rmsctl deployments cancel <id>

rmsctl updates list | show <id>                  # OTA-Updates
rmsctl updates deployments <robot-id>            # Update-Stand eines Roboters
rmsctl updates approve <id> --approver-id X
rmsctl updates deploy <id> <robot-id>
rmsctl updates rollback <id> <robot-id> --target-version X
```

### Skills, Embodiments & Prozesse

```bash
rmsctl skills list [--published]                 # VLA-Skills auflisten
rmsctl skills show <id>                          # Skill-Details
rmsctl skills for-robot <robot-id>               # Passende Skills
rmsctl skills compatible <id>                    # Passende Roboter
rmsctl skills check <id> <robot-id>              # Kompatibilitaet pruefen
rmsctl skills validate <id>
rmsctl skills publish <id> | deprecate <id> | archive <id>
rmsctl skills execute <id> --robot-id X [--prompt X]
rmsctl skills abort <id>

rmsctl embodiments list                          # Roboter-Typen auflisten
rmsctl embodiments show <tag>                    # Embodiment-Details & Config

rmsctl processes list | show <id>                # Prozess-Definitionen
rmsctl processes instances [--status X]          # Laufende Instanzen
rmsctl processes instance <id>
rmsctl processes tasks | task-queue
rmsctl processes start <id>
rmsctl processes pause <instance-id> | resume | cancel | retry
```

### Evaluation & Compliance

```bash
rmsctl eval episodes [--robot-id X] [--model-version X]
rmsctl eval success-rate [--robot-id X] [--model-version X] [--period 24h|7d|30d]
rmsctl eval episode <id>                         # Einzelne Episode
rmsctl eval errors [--robot-id X]                # Fehler-Aufschluesselung
rmsctl eval rewards                              # Reward-Model-Scores
rmsctl eval compare <versionA> <versionB> [--period 7d]
rmsctl eval run-hardware --robot-id X --skill-id Y [--episodes N]

rmsctl compliance dashboard                      # Compliance-Score
rmsctl compliance deadlines                      # Fristen
rmsctl compliance gaps [--summary]               # Offene Luecken
rmsctl compliance metrics | activity
rmsctl compliance documents                      # Ablaufende Dokumente
rmsctl compliance training [--summary]
rmsctl compliance inspections [--summary]
rmsctl compliance risk-assessments
```

### JSON-Ausgabe

Jeder Befehl unterstuetzt `--json`. Die Ausgabe ist der unveraenderte Server-Payload —
es wird nichts weggelassen und nichts hinzugefuegt:

```bash
rmsctl --json robots list
rmsctl --json datasets show <id>
rmsctl --json robots list | jq '.robots[].metadata.robotType'
rmsctl --json safety status | jq '.robots[] | select(.estop.status=="triggered")'
```

### Paginierung

Listen zeigen standardmaessig die erste Server-Seite. Wenn mehr Eintraege existieren als
angezeigt werden, sagt die Fussnote es und nennt das Flag:

```bash
$ rmsctl alerts list

  50 of 75 alert(s)
  [--] page 1 of 2 — use --page N, or --limit 75 for all
```

## VHS Tapes

Terminal-Recordings mit [VHS](https://github.com/charmbracelet/vhs):

```bash
vhs tapes/status.tape       # Flotten-Dashboard
vhs tapes/robots.tape       # Roboter-Liste & Details
vhs tapes/telemetry.tape    # Telemetrie-Ansicht
vhs tapes/pipeline.tape     # Daten- & Training-Pipeline
vhs tapes/upload.tape       # Datensatz direkt hochladen (braucht ein LeRobot-Verzeichnis)
vhs tapes/demo.tape         # Komplette Demo
```

## Stack

Rust, [clap](https://docs.rs/clap) (CLI), [reqwest](https://docs.rs/reqwest) (HTTP), [colored](https://docs.rs/colored) (Terminal-Output), [tokio](https://docs.rs/tokio) (Async), [sha2](https://docs.rs/sha2) (Pruefsummen beim Upload).
