> "[Bitcoin] takes advantage of the nature of information being easy to spread but hard to stifle." - Satoshi Nakamoto

While Bitcoin Core's mempool policies have been loosened over the years, there still remains some paternalism in what kinds of transactions Bitcoin Core allows. For example, Bitcoin Core maintains the pointless OP_Return size limit, even though it's just one of many ways to publish data in Bitcoin transactions; Libre Relay doesn't. Additionally, there's been constant pressure on Core to block more types of transactions for various reasons, such as censoring "spam".

Libre Relay is a fork of Bitcoin Core that does two things:
  - Removes paternalistic transaction filtering.
  - Peers with other Libre Relay nodes to ensure transactions that would have been blocked by Core can reach miners such as F2Pool and MARA anyway.

While this is of course good for people whose transactions are being blocked by Core, it's also good for Core itself: by having an alternative, when people try to pressure Core into blocking more transactions, Core can always point out that censorship doesn't work.

Finally, Libre Relay is also being used to develop Replace-By-Fee-Rate, a transaction pinning solution that mitigates pinning attacks on L2 protocols in a simple and effective way.