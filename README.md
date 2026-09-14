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
rmsctl datasets flagged <id>                     # Markierte Episoden
rmsctl datasets flag <id> <index> [--reason X]

rmsctl training list                             # Trainingsjobs auflisten
rmsctl training show <id>                        # Job-Details & Fortschritt
rmsctl training active | queue | workers         # Laufend, Warteschlange, Worker
rmsctl training start --config job.json          # Job starten
rmsctl training cancel <id> | retry <id>
rmsctl training export <id> | estimate <id>
```

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
vhs tapes/demo.tape         # Komplette Demo
```

## Stack

Rust, [clap](https://docs.rs/clap) (CLI), [reqwest](https://docs.rs/reqwest) (HTTP), [colored](https://docs.rs/colored) (Terminal-Output), [tokio](https://docs.rs/tokio) (Async).
