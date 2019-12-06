## Mirai
Hobby project for studying and developing peer-to-peer matchmaking and rollback netcode.

Current status: Somewhat functional matchmaking for two players when running everything on one machine.

Ultimate goal: Usable framework for small playercount games (e.g. <= 8).

### Components
#### mirai-core
Contains types and functionality that needs to be shared by two or more components of Mirai, including
- the message format used between the matchmaking client and matchmaking server

#### mirai-matchmaking-server
The matchmaking server should be relatively minimal and facilitate
- peer discovery
- storage and maintenance of ranking data 
- lobby discovery

#### mirai-matchmaking-client
The matchmaking client relies on the matchmaking server for peer/lobby discovery, but should handle
- matchmaking
- hosting and joining lobbies
- reporting match results

#### mirai-game-client
The game client handles all gameplay-related logic, including
- sending inputs
- receiving inputs
- detecting and adapting to drift, packet loss, latency spiking etc.

#### mirai-game
A sample game that should provide an interface for all the functionality provided by Mirai. To accomplish this, it should
- be able to quickly save and load the game state
