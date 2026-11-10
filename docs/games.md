# Supported games

Use the **id** (or an alias) as `game` in `lighthouse.toml`, or pick the game by name in `lighthouse setup`.
Run `lighthouse games` to print this list, or `lighthouse query <id> <host> [port]` to test a server.

The **port** is the default port players connect to. The **query** port is where Lighthouse asks for the
status; set `query_port` on a server if yours differs.

## Fully supported

Tested against live servers, with game-specific details (in-game time, blood moon, correct version).

| Game | id | Aliases | Protocol | Port | Query |
|---|---|---|---|---|---|
| 7 Days to Die | `seven_days_to_die` | `7dtd`, `7d2d`, `sdtd` | Steam query | 26900 | 26900 |
| Project Zomboid | `project_zomboid` | `pz`, `projectzomboid`, `zomboid` | Steam query | 16261 | 8766 |
| VEIN | `vein` |  | Steam query | 7777 | 7778 (port + 1) |

## Experimental

These use the generic adapter for their protocol. They should work, but their default ports come from
public query documentation and have not been checked on a live server yet. Reports and fixes are welcome.

| Game | id | Aliases | Protocol | Port | Query |
|---|---|---|---|---|---|
| Abiotic Factor | `abiotic_factor` | `abioticfactor` | Steam query | 27015 | 27015 |
| Action: Source | `action_source` | `actionsource` | Steam query | 27015 | 27015 |
| Alien Swarm | `alien_swarm` | `alienswarm` | Steam query | 27015 | 27015 |
| Aliens vs. Predator 2010 | `aliens_vs_predator` | `avp2010` | Steam query | 27015 | 27015 |
| America's Army: Proving Grounds | `americas_army_pg` | `aapg` | Steam query | 8777 | 27020 |
| American Truck Simulator | `american_truck_simulator` | `ats` | Steam query | 27015 | 27016 (port + 1) |
| ARK: Survival Evolved | `ark_survival_evolved` | `ase` | Steam query | 7777 | 27015 |
| ARMA 2 | `arma_2` | `arma2` | Steam query | 2302 | 2303 (port + 1) |
| ARMA 2: Operation Arrowhead | `arma_2_oa` | `a2oa` | Steam query | 2302 | 2303 (port + 1) |
| ARMA 3 | `arma_3` | `arma3` | Steam query | 2302 | 2303 (port + 1) |
| ARMA: Reforger | `arma_reforger` | `armareforger` | Steam query | 2001 | 17777 |
| Atlas | `atlas` |  | Steam query | 5761 | 57561 (port + 51800) |
| Avorion | `avorion` |  | Steam query | 27000 | 27020 (port + 20) |
| Ballistic Overkill | `ballistic_overkill` | `ballisticoverkill` | Steam query | 27015 | 27016 (port + 1) |
| Barotrauma | `barotrauma` |  | Steam query | 27015 | 27016 (port + 1) |
| Base Defense | `base_defense` | `basedefense` | Steam query | 27015 | 27015 |
| Battalion 1944 | `battalion_1944` | `battalion1944` | Steam query | 7777 | 7780 (port + 3) |
| Black Mesa | `black_mesa` | `blackmesa` | Steam query | 27015 | 27015 |
| Blade Symphony | `blade_symphony` | `bladesymphony` | Steam query | 27015 | 27015 |
| BrainBread 2 | `brainbread_2` | `brainbread2` | Steam query | 27015 | 27015 |
| Call of Duty: Black Ops 3 | `cod_black_ops_3` | `codbo3` | Steam query | 27017 | 27017 |
| Chivalry: Medieval Warfare | `chivalry` | `cmw` | Steam query | 7777 | 7779 (port + 2) |
| Codename CURE | `codename_cure` | `codenamecure` | Steam query | 27015 | 27015 |
| Colony Survival | `colony_survival` | `colonysurvival` | Steam query | 27004 | 27004 |
| Conan Exiles | `conan_exiles` | `conanexiles` | Steam query | 7777 | 27015 |
| Contagion | `contagion` |  | Steam query | 27015 | 27015 |
| Core Keeper | `core_keeper` | `corekeeper` | Steam query | 1234 | 1235 (port + 1) |
| Counter-Strike 1.6 | `counter_strike_1_6` | `counterstrike16` | Steam query | 27015 | 27015 |
| Counter-Strike 2 | `counter_strike_2` | `counterstrike2` | Steam query | 27015 | 27015 |
| Counter-Strike: Global Offensive | `counter_strike_go` | `csgo` | Steam query | 27015 | 27015 |
| Counter-Strike: Source | `counter_strike_source` | `css` | Steam query | 27015 | 27015 |
| Creativerse | `creativerse` |  | Steam query | 26900 | 26901 (port + 1) |
| Dark and Light | `dark_and_light` | `dal` | Steam query | 7777 | 27015 |
| Day of Defeat: Source | `day_of_defeat_source` | `dods` | Steam query | 27015 | 27015 |
| Day of Dragons | `day_of_dragons` | `dayofdragons` | Steam query | 7777 | 27015 |
| Day of Infamy | `day_of_infamy` | `doi` | Steam query | 27015 | 27015 |
| Days of War | `days_of_war` | `dow` | Steam query | 27015 | 27015 |
| DayZ Mod | `dayz_mod` | `dayzmod` | Steam query | 2302 | 2303 (port + 1) |
| Dino D-Day | `dino_d_day` | `ddd` | Steam query | 27015 | 27015 |
| Don't Starve Together | `dont_starve_together` | `dst` | Steam query | 10999 | 27016 |
| Dota 2 | `dota_2` | `dota2` | Steam query | 27015 | 27015 |
| Double Action: Boogaloo | `double_action` | `dab` | Steam query | 27015 | 27015 |
| Empyrion - Galactic Survival | `empyrion` | `egs` | Steam query | 30000 | 30001 (port + 1) |
| Enshrouded | `enshrouded` |  | Steam query | 15636 | 15637 |
| Euro Truck Simulator 2 | `euro_truck_simulator_2` | `ets2` | Steam query | 27015 | 27016 (port + 1) |
| Exfil | `exfil` |  | Steam query | 7777 | 27015 |
| Fistful of Frags | `fistful_of_frags` | `fof` | Steam query | 27015 | 27015 |
| FOUNDRY | `foundry` |  | Steam query | 27015 | 27015 |
| Garry's Mod | `garrys_mod` | `garrysmod` | Steam query | 27015 | 27015 |
| GoldenEye: Source | `goldeneye_source` | `goldeneyesource` | Steam query | 27015 | 27015 |
| Ground Breach | `ground_breach` | `groundbreach` | Steam query | 27015 | 27015 |
| GTA V (FiveM) | `fivem` | `gta5f`, `gta5_fivem` | FiveM HTTP | 30120 | 30120 |
| Half-Life 2: Deathmatch | `half_life_2_deathmatch` | `hl2d` | Steam query | 27015 | 27015 |
| Hell Let Loose | `hell_let_loose` | `hll` | Steam query | 27015 | 27015 |
| Hurtworld | `hurtworld` |  | Steam query | 12871 | 12881 |
| Icarus | `icarus` |  | Steam query | 27015 | 27015 |
| Insurgency | `insurgency` |  | Steam query | 27015 | 27015 |
| Insurgency: Sandstorm | `insurgency_sandstorm` | `insurgencysandstorm` | Steam query | 27015 | 27016 (port + 1) |
| Just Cause 3 - Multiplayer | `just_cause_3_mp` | `jc3m` | Steam query | 4200 | 4201 (port + 1) |
| Killing Floor 2 | `killing_floor_2` | `killingfloor2` | Steam query | 7777 | 27015 |
| Kreedz Climbing | `kreedz_climbing` | `kreedzclimbing` | Steam query | 27015 | 27015 |
| Left 4 Dead | `left_4_dead` | `l4d` | Steam query | 27015 | 27015 |
| Left 4 Dead 2 | `left_4_dead_2` | `l4d2` | Steam query | 27015 | 27015 |
| Medieval Engineers | `medieval_engineers` | `medievalengineers` | Steam query | 27015 | 27015 |
| Minecraft (Bedrock Edition) | `minecraft_bedrock` | `mbe`, `bedrock` | Minecraft Bedrock ping | 19132 | 19132 |
| Minecraft (Java Edition) | `minecraft` | `mc`, `minecraft_java` | Minecraft Java ping | 25565 | 25565 |
| Mordhau | `mordhau` |  | Steam query | 7777 | 27015 |
| Myth of Empires | `myth_of_empires` | `moe` | Steam query | 12888 | 12888 |
| No More Room in Hell | `no_more_room_in_hell` | `nmrih` | Steam query | 27015 | 27015 |
| Nova-Life: Amboise | `nova_life` | `nla` | Steam query | 27015 | 27015 |
| Nuclear Dawn | `nuclear_dawn` | `nucleardawn` | Steam query | 27015 | 27015 |
| Onset | `onset` |  | Steam query | 7777 | 7776 (port - 1) |
| Operation: Harsh Doorstop | `operation_harsh_doorstop` | `ohd` | Steam query | 7777 | 27005 |
| PixARK | `pixark` |  | Steam query | 7777 | 27015 |
| Post Scriptum | `post_scriptum` | `postscriptum` | Steam query | 10037 | 10037 |
| Primal Carnage: Extinction | `primal_carnage_extinction` | `pce` | Steam query | 7777 | 27015 |
| Project Cars | `project_cars` | `projectcars` | Steam query | 27015 | 27016 (port + 1) |
| Project Cars 2 | `project_cars_2` | `projectcars2` | Steam query | 27015 | 27016 (port + 1) |
| Quake Live | `quake_live` | `quakelive` | Steam query | 27960 | 27960 |
| Red Dead Redemption 2 (RedM) | `redm` | `rdr2r`, `rdr2_redm` | FiveM HTTP | 30120 | 30120 |
| Red Orchestra 2 | `red_orchestra_2` | `redorchestra2` | Steam query | 7777 | 27015 |
| rFactor 2 | `rfactor_2` | `rfactor2` | Steam query | 64299 | 64299 |
| Rising Storm 2: Vietnam | `rising_storm_2` | `rs2v` | Steam query | 27015 | 27015 |
| Rising World | `rising_world` | `risingworld` | Steam query | 4255 | 4254 (port - 1) |
| Risk of Rain 2 | `risk_of_rain_2` | `ror2` | Steam query | 27015 | 27016 (port + 1) |
| Rust | `rust` |  | Steam query | 28015 | 28015 |
| Sons of the Forest | `sons_of_the_forest` | `sotf` | Steam query | 8766 | 27016 |
| Soulmask | `soulmask` |  | Steam query | 8777 | 27015 |
| Space Engineers | `space_engineers` | `spaceengineers` | Steam query | 27015 | 27015 |
| Starbound | `starbound` |  | Steam query | 21025 | 21025 |
| Survive the Nights | `survive_the_nights` | `stn` | Steam query | 7950 | 7951 (port + 1) |
| Sven Coop | `sven_coop` | `svencoop` | Steam query | 27015 | 27015 |
| Synergy | `synergy` |  | Steam query | 27015 | 27015 |
| Team Fortress 2 | `team_fortress_2` | `teamfortress2` | Steam query | 27015 | 27015 |
| Team Fortress Classic | `team_fortress_classic` | `tfc` | Steam query | 27015 | 27015 |
| The Cenozoic Era | `the_cenozoic_era` | `tce` | Steam query | 7777 | 7778 (port + 1) |
| The Forest | `the_forest` | `theforest` | Steam query | 27015 | 27016 (port + 1) |
| The Front | `the_front` | `thefront` | Steam query | 27015 | 27015 |
| The Isle | `the_isle` | `theisle` | Steam query | 7707 | 7708 (port + 1) |
| Tower Unite | `tower_unite` | `towerunite` | Steam query | 27015 | 27015 |
| Unturned | `unturned` |  | Steam query | 27015 | 27016 (port + 1) |
| V Rising | `v_rising` | `vrising` | Steam query | 27015 | 27015 |
| Valheim | `valheim` |  | Steam query | 2456 | 2457 (port + 1) |
| Zombie Panic: Source | `zombie_panic_source` | `zps` | Steam query | 27015 | 27015 |

107 games in total.
