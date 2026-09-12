Monitor Bitcoin addresses in the mempool and on-chain using the mempool.space API and websocket.

Track balance changes, get telegram notifications, and manage expectations for multiple addresses.

Features:
  - Build list of addresses in collections to sum-up balances accross wallets
  - Subscribes to mempool.space (or locally hosted) websocket for real-time mempool activity
  - Double checks data against mempool.space (or locally hosted) API on a configured interval
  - Track both on-chain and mempool activity
  - Configure auto-acceptance or alert mode of changes (chain_in, chain_out, mempool_in, mempool_out)
    - by default, incoming transactions are auto-accepted, outgoing transactions are alerted
    - all activity is alerted to a configured telegram connection once (even when auto-accepted)
  - Option to use your own local node for privacy


Recommended to have Mempool + Fulcrum to run privately but not required.