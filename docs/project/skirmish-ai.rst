Planned: skirmish AI (M8)
=========================

The AI is a virtual player that issues ordinary commands. Nothing here is
implemented yet.

Design
------

* A new ``rts_ai`` crate, created when M8 starts. It reads simulation state
  and emits ``Command`` values. It never depends on the client.
* The AI runs on the server and submits commands through the same
  authoritative validation as human input.
* Because the server's turns carry the AI's commands to every client, AI
  decision code does **not** need to be deterministic. Only its commands
  enter the simulation.

.. kroki::
   :type: plantuml
   :caption: An AI player in a server room.
   :align: center

   @startuml
   skinparam backgroundColor transparent
   skinparam componentStyle rectangle
   node "rts-server room" {
     [Authoritative Sim] as sim
     [rts_ai player] as ai
     [Input validation] as val
   }
   actor "Human client" as human
   sim --> ai : read state
   ai --> val : Commands for its slot
   human --> val : Input
   val --> sim : Turn every 50 ms
   sim --> human : Turn + hash
   @enduml
