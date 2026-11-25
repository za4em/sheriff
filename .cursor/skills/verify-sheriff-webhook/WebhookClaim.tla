---- MODULE WebhookClaim ----
\* One retained decision, two workers, two Sheriff instances.
\*
\* ReadPending     worker.rs score_group: group load, this decision is still pending
\* ReadDone        worker.rs score_group: group load, this decision already left pending, publish false
\* MarkReady       decisions/repository.rs mark_ready: UPDATE ... WHERE status = pending
\* Publish         cache.rs ready_decision: XADD sheriff.ready with completed_now
\* Observe         cache/reader.rs: each instance reads the stream (not a consumer group)
\* Claim           cache.rs claim_webhook: SET sheriff.hook.<id> NX EX
\* Post            engine.rs observe then webhook: deliver_decision only after a won claim
\*
\* UseClaim is TRUE for the code. FALSE drops the NX, which is the broken variant.

EXTENDS Integers, FiniteSets, Sequences

CONSTANTS Instances, Workers, UseClaim

VARIABLES
  status,
  readPending,
  won,
  lost,
  published,
  stream,
  cursor,
  seenTrue,
  claim,
  mayPost,
  posts

vars == <<status, readPending, won, lost, published, stream, cursor, seenTrue, claim, mayPost, posts>>

Init ==
  /\ status = "pending"
  /\ readPending = {}
  /\ won = {}
  /\ lost = {}
  /\ published = {}
  /\ stream = <<>>
  /\ cursor = [i \in Instances |-> 0]
  /\ seenTrue = {}
  /\ claim = "none"
  /\ mayPost = {}
  /\ posts = {}

started(w) == w \in (readPending \cup won \cup lost \cup published)

ReadPending(w) ==
  /\ status = "pending"
  /\ ~started(w)
  /\ readPending' = readPending \cup {w}
  /\ UNCHANGED <<status, won, lost, published, stream, cursor, seenTrue, claim, mayPost, posts>>

ReadDone(w) ==
  /\ status = "ready"
  /\ ~started(w)
  /\ lost' = lost \cup {w}
  /\ UNCHANGED <<status, readPending, won, published, stream, cursor, seenTrue, claim, mayPost, posts>>

MarkReady(w) ==
  /\ w \in readPending
  /\ readPending' = readPending \ {w}
  /\ IF status = "pending"
     THEN /\ status' = "ready"
          /\ won' = won \cup {w}
          /\ UNCHANGED lost
     ELSE /\ lost' = lost \cup {w}
          /\ UNCHANGED <<status, won>>
  /\ UNCHANGED <<published, stream, cursor, seenTrue, claim, mayPost, posts>>

Publish(w) ==
  /\ w \in (won \cup lost)
  /\ w \notin published
  /\ stream' = Append(stream, w \in won)
  /\ published' = published \cup {w}
  /\ UNCHANGED <<status, readPending, won, lost, cursor, seenTrue, claim, mayPost, posts>>

Observe(i) ==
  /\ cursor[i] < Len(stream)
  /\ cursor' = [cursor EXCEPT ![i] = cursor[i] + 1]
  /\ IF stream[cursor[i] + 1]
     THEN seenTrue' = seenTrue \cup {i}
     ELSE UNCHANGED seenTrue
  /\ UNCHANGED <<status, readPending, won, lost, published, stream, claim, mayPost, posts>>

Claim(i) ==
  /\ i \in seenTrue
  /\ seenTrue' = seenTrue \ {i}
  /\ IF UseClaim
     THEN IF claim = "none"
          THEN /\ claim' = i
               /\ mayPost' = mayPost \cup {i}
          ELSE UNCHANGED <<claim, mayPost>>
     ELSE /\ mayPost' = mayPost \cup {i}
          /\ UNCHANGED claim
  /\ UNCHANGED <<status, readPending, won, lost, published, stream, cursor, posts>>

Post(i) ==
  /\ i \in mayPost
  /\ mayPost' = mayPost \ {i}
  /\ posts' = posts \cup {i}
  /\ UNCHANGED <<status, readPending, won, lost, published, stream, cursor, seenTrue, claim>>

Done ==
  /\ \A w \in Workers : w \in published
  /\ \A i \in Instances : cursor[i] = Len(stream)
  /\ seenTrue = {}
  /\ mayPost = {}
  /\ UNCHANGED vars

Next ==
  \/ \E w \in Workers : ReadPending(w) \/ ReadDone(w) \/ MarkReady(w) \/ Publish(w)
  \/ \E i \in Instances : Observe(i) \/ Claim(i) \/ Post(i)
  \/ Done

Spec == Init /\ [][Next]_vars

TypeOK ==
  /\ status \in {"pending", "ready"}
  /\ readPending \subseteq Workers
  /\ won \subseteq Workers
  /\ lost \subseteq Workers
  /\ published \subseteq (won \cup lost)
  /\ won \cap lost = {}
  /\ won \cap readPending = {}
  /\ lost \cap readPending = {}
  /\ stream \in Seq(BOOLEAN)
  /\ cursor \in [Instances -> Nat]
  /\ \A i \in Instances : cursor[i] <= Len(stream)
  /\ seenTrue \subseteq Instances
  /\ mayPost \subseteq Instances
  /\ posts \subseteq Instances
  /\ claim \in (Instances \cup {"none"})

\* The SQL update matches at most one worker.
OneCompletion == Cardinality(won) <= 1

\* A retained decision is handed to deliver_decision by at most one instance.
AtMostOneWebhook == Cardinality(posts) <= 1

====
