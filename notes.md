Bad Commit:
    ac42b1112a3035c31a3d719fe3549c2cd9d818a0 is the first bad commit
    commit ac42b1112a3035c31a3d719fe3549c2cd9d818a0
    Author: Brendan Nolan <bnolan23@gmail.com>
    Date:   Thu Sep 14 08:17:07 2023 +0100
    
        Make Main Loop Handle Internal Events Only
    
        Don't consider it an Event (for the main loop to be concerned with) if
        something is received from outside - that is something which the p2p
        module can deal with.
    
     src/main.rs | 37 +++++++++----------------------------
     src/p2p.rs  | 52 +++++++++++++++++++++++-----------------------------
     2 files changed, 32 insertions(+), 57 deletions(-)

