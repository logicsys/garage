Garage Object Storage: 2.0 update and best practices
====================================================

> Garage (project website) is a versatile object storage software, focused on decentralized and geo-distributed deployments. The software has been developed under the AGPL for more than 5 years and is now reaching maturity.
> This talk will cover development and new features of the 2.x releases since the last FOSDEM talk (2024), best practices for administrators, available UIs, and a small tutorial on how to migrate from minio.

- Who is Deuxfleurs?
- What is Garage?
- What Garage is NOT
- New features in Garage 2.x
- Best practices for administrators
  - What you should know
    - Garage doesn't support TLS, you'll have to put you reverse proxy in front
    - Anonymous access works by exposing the content as a website
    - Layouts are managed _manually_
    - Changing the number of replica in a live cluster is possible but not supported
    - Mind the region! It default to `garage` and a lot of workloads default to `us-east-1`
  - About the hardware
    - Get a write-intensive disk for the `/metadata`
      - ideally on a RAID1 CoW fs with compression (zfs or btrfs)
    - Use garage multi-hdd over RAID for the `/data` folder, each drive on xfs
  - Single node
    - Garage was meant to be a distributed system form the start
    - We do NOT advise to use a single node for production
    - If you do please ensure you have backups (especially for metadata)
    - You may want to enable fsync and use sqlite
      - More resistant to corruption than LMDB
    - Get a UPS if you have flaky power
  - Multi-node setup
    - Try to have a geo-distributed zones
    - You can have multiple nodes per zone to add more capacity
    - Deuxfleurs has been running a 8-nodes cluster (3+3+2) over retail fiber (~10ms site-to-site latency) for close to 5 years now
    - Keep in mind your available network and IO bandwidth
      - _Rebalancing a cluster can take multiple weeks_ with large HDDs and slow network links
  - Choosing a metadata engine
    - LMDB
    - sqlite
    - fjall (experimental)
  - Using the Admin API
    - Everything that can be done with the CLI can be done though the HTTP admin API
    - Dedicated port, one secret (bearer)
    - As of garage 2.1, the CLI still use the node-to-node RPC
  - Backing up Garage (data and metadata)
  - How to make sense of Garage metrics
  - Debugging & repairing garage
    - Start by setting
    - Use the repair commands
- Available UIs
- Upcoming roadmap
- How to migrate from minio
  - Recreate your buckets and keys
  - Transfer your data
- Q&A

Issues

resync-tranquility
Consistency-mode consistent not degraded
K8s + consul
WebUI intégré dans garage

Ajouter un flag pour l'import de clef secret externe
https://git.deuxfleurs.fr/Deuxfleurs/garage/issues/1262

Roadmap

Admin API -> garage stats needs to change

CONTRIBUTING.md
MAINTAINERS.md

Impliqué sur les changement de BDD, qorum, breaking change, modèle de données
