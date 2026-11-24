# Étude : envoyer des fichiers à quelqu'un d'autre, de pair à pair

Novembre 2026. Étude seulement : rien n'est encore implémenté.

## Le besoin

- Envoyer un fichier ou un dossier, **sans limite de taille**, à une personne qui n'est pas sur le même réseau.
- Aucun compte, aucun serveur de stockage : les données vont directement d'un ordinateur à l'autre.
- Chiffré de bout en bout, vérifié à l'arrivée, et capable de **reprendre** après une coupure.
- Fidèle aux règles de Chud Copy : rien ne part ni n'arrive sans un geste explicite des deux côtés, et rien n'est écrit en dehors du dossier choisi par le destinataire.

## La difficulté : traverser les box internet

Deux ordinateurs derrière des box (NAT) ne peuvent pas s'appeler directement. Il faut :

1. un **rendez-vous** pour qu'ils se trouvent ;
2. le **perçage de trous** (« hole punching ») pour ouvrir une connexion directe, qui réussit dans la grande majorité des cas ;
3. un **relais** de secours quand le perçage échoue (certains réseaux d'entreprise ou mobiles). Le relais ne voit que du trafic chiffré.

## Solutions examinées

| Solution | Langage, licence | Traversée des box | Reprise, vérification | Infrastructure | Verdict |
| --- | --- | --- | --- | --- | --- |
| **iroh + iroh-blobs** | Rust, MIT ou Apache-2.0. iroh 1.4.1 (stable depuis la 1.0 de juin 2026), iroh-blobs 0.104.1 | QUIC, perçage de trous, relais en secours | Oui : contenu adressé par empreinte BLAKE3, vérifié au fil de l'eau, reprise native | Relais publics de n0 ; relais auto-hébergeable | **Recommandé** |
| magic-wormhole (crate Rust) | Rust, **EUPL-1.2** | Serveur de rendez-vous + relais de transit | Reprise limitée | Serveurs publics de la communauté | Licence plus contraignante, protocole moins adapté aux très gros dossiers |
| WebRTC | Bibliothèques variées | ICE, STUN, TURN | À construire | Serveur de signalisation et serveur TURN à héberger | Beaucoup d'infrastructure à maintenir |
| libp2p | Rust, MIT | Oui | À construire au-dessus | Nœuds relais à prévoir | Puissant mais lourd pour ce seul besoin |

## Recommandation : iroh et iroh-blobs

Pourquoi :

- **Même langage que le cœur** de Chud Copy (Rust), et licence compatible.
- **Taille illimitée et reprise** : un fichier est découpé et vérifié par morceaux (BLAKE3). Une coupure reprend là où elle s'est arrêtée, et une donnée altérée en route est détectée et refusée.
- **Connexion directe** dans la plupart des cas, relais chiffré sinon. Chaque appareil est identifié par une clé publique ; la connexion est chiffrée de bout en bout (QUIC, TLS 1.3).
- **Pas de serveur à nous** pour commencer : les relais publics de n0 suffisent. Un relais auto-hébergé reste possible plus tard si le volume l'exige.

## Comment ça se passerait pour l'utilisateur

Un nouvel outil « Envoyer à quelqu'un » :

1. **L'expéditeur** choisit un fichier ou un dossier. Chud Copy calcule son empreinte et affiche un **code de partage** (un « ticket » à copier, ou un QR code), valable tant que la fenêtre reste ouverte.
2. Il transmet ce code comme il veut : message, mail, téléphone.
3. **Le destinataire** colle le code dans « Recevoir », voit le nom et la taille de ce qu'on lui envoie, choisit le dossier d'arrivée et **accepte**. Rien n'arrive sans cette acceptation.
4. Les deux voient la vitesse, le temps restant, et si la connexion est directe ou relayée. Une coupure se reprend en relançant avec le même code.

Garde-fous :

- L'envoi s'arrête quand l'expéditeur ferme l'outil ou l'application : rien ne reste partagé en arrière-plan.
- Côté destinataire, rien n'est remplacé : un fichier déjà présent garde son nom, la copie reçue en prend un autre, comme pour la restauration.
- Les deux ordinateurs doivent rester allumés pendant le transfert.

## Coûts et risques

- **Taille de l'application** : quelques mégaoctets de plus.
- **Évolution d'iroh-blobs**, encore en version 0.x : son API peut changer d'une version à l'autre. Il faudra figer la version et tester chaque mise à jour.
- **Relais publics** : gratuits mais partagés ; un transfert relayé est plus lent qu'un transfert direct. Si l'usage grandit, héberger son propre relais.
- **Pare-feu** : Windows peut demander d'autoriser l'application la première fois.

## Estimation

Une version complète, avec module dans le cœur, deux écrans (envoyer, recevoir), progression, reprise et tests (dont un transfert réel entre deux instances isolées sur des dossiers temporaires) représente une version entière de Chud Copy, comparable à l'ajout des outils de la 1.6.
