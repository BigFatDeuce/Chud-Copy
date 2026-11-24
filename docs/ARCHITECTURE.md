# Architecture de Chud Copy

## Choix du moteur

| Critère | rclone | rsync | Robocopy | Moteur Rust maison |
| --- | --- | --- | --- | --- |
| Windows et Linux | Oui, binaire natif | Pas de version Windows native | Windows seul | Oui |
| Licence pour redistribuer | MIT | GPLv3, contraignante | Propriétaire, non redistribuable | Libre |
| Fiabilité | Éprouvé à grande échelle | Éprouvé | Éprouvé | À prouver entièrement |
| Prévisualisation exploitable | `--dry-run --combined` | Sortie texte à interpréter | Sortie texte localisée | À écrire |
| Statistiques en direct | JSON structuré | Texte | Texte | À écrire |
| Chemins longs sous Windows | Automatique | Sans objet | Oui | À écrire |
| Intégrité | Empreinte pendant le transfert, `check` | Somme de contrôle | Aucune | À écrire |
| Distribution | Un binaire statique, sans dépendance | Dépendances Cygwin sous Windows | Déjà présent | Intégré |

**Décision : rclone 1.75.2 embarqué**, version et empreintes épinglées dans `scripts/fetch-rclone.mjs`. C'est la seule option à cocher toutes les cases, et elle évite de réinventer un moteur sans nécessité.

### Intégration

Chaque liaison lance son propre processus rclone en ligne de commande. Le mode serveur `rcd` a été écarté : il n'applique pas `--retries`, ne propose pas `--combined`, et un service permanent contredit la philosophie du logiciel.

| Besoin | Solution | Vérifié par |
| --- | --- | --- |
| Progression réelle | Statistiques JSON de rclone toutes les 500 ms | Test `progress_comes_from_the_engine` |
| Total connu dès le départ | `--check-first` | Test à 100 000 fichiers |
| Pause | Suspension du processus, `NtSuspendProcess` ou `SIGSTOP` | Test `pause_really_stops_the_engine` |
| Arrêt propre | `core/quit` sur un port local éphémère à secret aléatoire, arrêt forcé après 8 s | Test `cancelling_leaves_no_partial_file` |
| Arrêt avec l'application | Job Object Windows, `PR_SET_PDEATHSIG` sous Linux | Parcours « fermeture brutale » |
| Isolation | Aucun fichier de configuration, variables `RCLONE_*` retirées | Test `engine_environment_cannot_change_an_operation` |

Sous Windows, le moteur est lancé avec `GODEBUG=winsymlink=0`. Sans ce réglage, rclone ignore sans aucun message les fichiers gérés par un service cloud comme MEGA, Nextcloud ou OneDrive : depuis Go 1.23, ces points d'analyse ne sont plus des fichiers ordinaires. Lire un fichier cloud absent du disque déclenche son téléchargement par le logiciel de synchronisation.

En comparaison par taille et date, une tolérance d'une seconde s'applique (`--modify-window 1s`). MEGA, Nextcloud, FAT et exFAT arrondissent les dates : sans elle, chaque écart forcerait la relecture complète du fichier sur les deux disques. Le mode « Contenu complet » reste disponible pour comparer sans tolérance.

`--metadata` n'est volontairement pas utilisé. Il fait échouer la copie d'un fichier dès que son propriétaire ne peut pas être rétabli, par exemple quand un utilisateur normal copie le fichier d'un autre. Les attributs sont restaurés par `core/src/attrs.rs`, après la copie et sans pouvoir la compromettre.

## Organisation

```
src/                    Interface React et TypeScript
  cosmos/engine.ts      Ciel procédural et fluide WebGL 2
  views/                Accueil, éditeur, analyse, exécution, historique, réglages
  lib/tasks.ts          Logique de l'éditeur, séparation des sources en sous-dossiers
  lib/machines.ts       États de l'interface et leurs règles, voir « Machines à états »
src-tauri/
  src/lib.rs            Commandes Tauri, sans logique métier
  core/                 Crate chud-core, sans dépendance à l'interface
    engine/mod.rs       Contrat Engine, frontière du moteur
    engine/rclone.rs    Implémentation rclone
    validate.rs         Garde-fous avant toute opération
    plan.rs             Analyse et requêtes paginées sur les résultats
    runner.rs           Exécution et contrat de suppression
    schedule.rs         Répartition sur les disques
    store.rs            Persistance atomique
    history.rs          Rapports et journaux
tests/e2e/              Parcours sur la vraie application
docker/                 Compilation Linux et validation sur Fedora
```

Remplacer le moteur revient à fournir une autre implémentation du trait `Engine`. L'interface, la validation, l'historique et les garde-fous n'en dépendent pas.

## Sécurité des données

Les garde-fous sont dans le cœur, pas dans l'interface : un défaut d'affichage ne peut pas les contourner.

- **Chemins.** Détection des chemins identiques, imbriqués ou se chevauchant après résolution des liens, de la casse Windows et du préfixe `\\?\`. rclone accepte de copier un dossier dans lui-même : Chud Copy le refuse.
- **Sources.** Une destination ne peut jamais empiéter sur la source d'une autre liaison.
- **Contrat de suppression.** Un miroir exige une analyse de la version exacte de la tâche. La liste des suppressions est recomparée juste avant d'écrire, et `--max-delete` reste un second filet. Le nombre seul ne suffit pas : un fichier arrivé après l'analyse pourrait prendre la place d'un fichier validé.
- **Vue du moteur recoupée.** Chaque analyse compte les fichiers de la source sans passer par le moteur. S'il en voit moins, un avertissement s'affiche et le miroir est bloqué, car il supprimerait la copie de fichiers encore présents.
- **Source vide.** Une source vide face à une destination pleine ressemble à un disque non monté : elle est signalée et demande une confirmation renforcée.
- **Dossiers protégés.** Un miroir ne peut pas viser un dossier système ou personnel.
- **Fichiers incomplets.** rclone écrit dans un `.partial` renommé à la fin. Après un arrêt brutal, le reste est signalé à l'analyse.
- **Erreurs.** Si une liaison rencontre une erreur, rclone annule ses suppressions et les autres liaisons continuent.

## Effets visuels

Les effets (`src/effects/`) ne s'animent que si le réglage d'animation est actif et que le système ne demande pas de réduire les mouvements. Ils n'utilisent pas la capture de page expérimentale de certaines bibliothèques, absente de WebView2 et de WebKitGTK : nuages d'analyse en canvas 2D, brasier de copie et laser derrière le titre en canvas 2D (un seul contexte WebGL, celui du ciel), dont l'ardeur suit le débit réel, pluie de glyphes après trois minutes sans geste, glitch au clic en surcouche (la zone cliquable du bouton ne change jamais), mode discret par filtre SVG. L'ordre des tâches est enregistré dans les réglages (`taskOrder`), leur couleur dans la tâche elle-même.

## File d'attente

Une seule opération s'exécute à la fois. Celles que l'utilisateur met en file (`core/src/queue.rs`) démarrent ensuite dans l'ordre. La file reste en mémoire et n'est jamais enregistrée : rien ne repart au lancement suivant. Une annulation la met en pause. Une opération qui ne peut pas démarrer y reste avec sa raison, sans bloquer les suivantes. Un miroir n'y entre qu'avec son analyse validée, et ses suppressions sont recomparées juste avant d'écrire, comme pour un lancement direct.

## Machines à états

Tout ce qui a un cycle de vie est décrit une seule fois, par un type qui n'admet que les états possibles et par des fonctions pures qui répondent aux questions des écrans. Une vue ne recompose jamais ces réponses : elle les demande. Changer une règle se fait donc à un seul endroit, et un état impossible ne se compile pas.

| Machine | États | Où elle vit | Qui la fait avancer |
| --- | --- | --- | --- |
| Opération | `running` ⇄ `paused`, → `cancelling` → `cancelled`, → `completed`, `completedWithErrors`, `failed` ; `interrupted` après une fermeture brutale | `RunStatus` dans `core/src/history.rs`, relu par `isLive`, `liveRun`, `ending` | Le cœur seul (`runner.rs`) ; l'interface reçoit `run:update` |
| Consigne | `RUNNING` ⇄ `PAUSED`, → `CANCELLED` sans retour | `Control` dans `core/src/engine/mod.rs` | Les commandes `run_pause`, `run_resume`, `run_cancel` |
| Liaison | `waiting` → `copying` → `verifying` → `done`, `failed`, `cancelled` ; `skipped` dès le départ | `LinkPhase` dans `core/src/history.rs` | `runner.rs` |
| File d'attente | libre ou retenue (`held`) ; chaque entrée est en attente ou refusée avec sa raison | `core/src/queue.rs`, testé | Les commandes `queue_*`, et la fin de chaque opération |
| Lancement | `start`, `queue`, `queued` | `launchChoice` et `launch` du contexte | Déduit de l'opération et de la file |
| Analyse | `running` → `done` avec son plan, ou `failed` avec sa raison ; phase affichée `running`, `failed`, `stopped`, `ready` | `AnalysisEntry`, `analysisPhase` | `startAnalysis` du contexte |
| Verdict d'un plan | `failed`, `unseen`, `stopped`, `upToDate`, `ready`, `readyArchive`, `readyDelete` | `planVerdict`, `planRunnable` | Déduit du plan et du mode |
| Outil ou contrôle | `idle` → `running` → `done` avec son résultat → `idle` | `Job`, `useJob`, `CheckState` | L'utilisateur ; l'état survit au changement d'onglet |
| Barre de titre | `native` ou `custom` | `NATIVE_CHROME` dans `src-tauri/src/lib.rs`, publié par `AppInfo.chrome` | Fixé au démarrage |
| Extinction en fin de copie | `off` → `armed` → `counting` → extinction ; retour à `armed` si une opération repart, à `off` sur annulation | `Shutdown`, `shutdownStep` ; jamais enregistrée | La case de l'accueil ou de l'exécution, puis l'opération et la file ; le cœur revérifie avant `system_power_off` |
| Recherche de mise à jour | `idle`, `checking`, `latest`, `available`, `failed` | `UpdateState` dans `views/Settings.tsx` | L'utilisateur |

Les fonctions de l'interface sont dans `src/lib/machines.ts`. Trois règles tiennent l'ensemble :

- **Le cœur fait autorité.** L'interface ne décide jamais qu'une opération est finie ou qu'un miroir peut partir : elle lit ce que le cœur publie, et le cœur revérifie tout au lancement.
- **Un état, un type.** Une analyse terminée porte toujours son plan, un travail terminé toujours son résultat. Pas de couple « en cours » et « résultat » à tenir cohérent à la main.
- **Une question, une fonction.** « Une opération occupe-t-elle le moteur ? » se demande à `isLive`, nulle part ailleurs.

Restent volontairement locaux, parce qu'ils ne concernent qu'un écran : l'ouverture d'une fenêtre de confirmation, le glisser-déposer des cartes de l'accueil, les modifications non enregistrées de l'éditeur.

### Fenêtre sous Linux

Le bureau dessine la barre de titre (`NATIVE_CHROME`). Elle est demandée à la création de la fenêtre et jamais après : sous Wayland, une fenêtre créée sans décorations reçoit une barre vide, et l'activer ensuite n'y ajoute aucun bouton. Là où GTK dessine la barre lui-même, comme sous GNOME, `complete_window_buttons` ajoute réduire et agrandir quand le bureau ne demande que fermer. `docker/wayland-shot.sh` capture la fenêtre sous Wayland pour le vérifier. Les listes déroulantes sont dessinées par l'application (`Select` dans `components/ui.tsx`) : celles de WebKitGTK sont peintes par GTK, hors de portée des styles, et sortaient en texte clair sur fond blanc.

## Fichiers inutiles

L'option « Ignorer les fichiers inutiles » est décochée par défaut. Sa liste est dans `core/src/filters.rs` : profils et caches Windows, mémoire virtuelle, résidus macOS et Linux, téléchargements inachevés, journaux de synchronisation. Elle exclut volontairement ce qui peut contenir de vraies données. Les fichiers écartés restent visibles dans la catégorie « Ignorés » de l'analyse.

## Langues

Le cœur Rust n'écrit aucune phrase destinée à l'utilisateur : il renvoie des codes avec leurs paramètres, traduits par l'interface. Tous les textes vivent dans `src/i18n`, un fichier par langue, et `fr.ts` sert de référence.

- TypeScript refuse de compiler s'il manque une clé dans une langue.
- `tests/i18n-check.mjs` vérifie que chaque traduction garde les variables du français, comme `{path}`. Il fait partie de `npm run build`.
- `tests/e2e/languages.mjs` parcourt les vues dans chaque langue et signale les débordements, les mots français oubliés et les chemins retournés.

Les polices embarquées ne couvrent que le latin et le cyrillique partiel : le chinois, le hindi, le bengali, l'arabe et l'ourdou utilisent les polices du système. L'arabe et l'ourdou passent l'interface de droite à gauche, sauf la carte des liaisons, les chemins et les tailles, qui gardent leur sens de lecture.

## Fond animé

La simulation reprend la méthode de WebGL Fluid Simulation de Pavel Dobryakov (MIT), réécrite pour un usage permanent derrière une application : basse résolution, cadence plafonnée, ciel calculé une seule fois, arrêt quand la fenêtre est masquée. Les panneaux n'utilisent pas `backdrop-filter`, qui imposerait une passe GPU plein écran à chaque image. Pendant un transfert, l'intensité suit le débit réel du moteur.

Mesuré au repos sur Ryzen 7 9800X3D, charge CPU rapportée à un seul cœur :

| Qualité | CPU |
| --- | --- |
| Désactivé | 1,7 % |
| Économe | 6,2 % |
| Équilibré | 8,1 % |
| Élevé | 8,9 % |

Sans WebGL 2, l'interface garde un dégradé statique.
