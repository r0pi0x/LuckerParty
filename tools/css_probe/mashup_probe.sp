// mashup_probe: replays per-tick inputs on a CS:S bot and logs the result,
// so mashup's Source movement can be compared with the real game tick for
// tick (docs/OBSERVABILITY.md, "Tick-exact CS:S comparisons").
//
// Server command:  mashup_run <input file> <output file>   (paths relative
// to the game folder). Runs on the first living bot; use bot_zombie 1 so
// the bot's own AI doesn't fight the inputs.
//
// Input file: one optional "start x y z pitch yaw vx vy vz" line, then one
// line per tick: "buttons forwardmove sidemove pitch yaw".
// Output file: one line per tick, "tick x y z vx vy vz onground ducked";
// tick 0 is the state after the bot was placed at the start (one idle
// tick), tick n the state after input line n.

#include <sourcemod>
#include <sdktools>

public Plugin myinfo = {
    name = "mashup probe",
    author = "mashup",
    description = "Replays per-tick inputs on a bot and logs its movement",
    version = "1",
};

#define MAX_TICKS 16384

int g_buttons[MAX_TICKS];
float g_forward[MAX_TICKS];
float g_side[MAX_TICKS];
float g_pitch[MAX_TICKS];
float g_yaw[MAX_TICKS];
int g_count;

float g_startPos[3];
float g_startAng[3];
float g_startVel[3];
bool g_hasStart;

int g_bot;
File g_out;
// -1: place the bot on its next command; then the index of the next input.
int g_cursor = -1;

public void OnPluginStart()
{
    RegServerCmd("mashup_run", Command_Run, "mashup_run <input> <output>");
}

int FindBot()
{
    for (int i = 1; i <= MaxClients; i++) {
        if (IsClientInGame(i) && IsFakeClient(i) && IsPlayerAlive(i)) {
            return i;
        }
    }
    return 0;
}

public Action Command_Run(int args)
{
    char inPath[PLATFORM_MAX_PATH];
    char outPath[PLATFORM_MAX_PATH];
    GetCmdArg(1, inPath, sizeof(inPath));
    GetCmdArg(2, outPath, sizeof(outPath));

    File f = OpenFile(inPath, "r");
    if (f == null) {
        PrintToServer("mashup: cannot open %s", inPath);
        return Plugin_Handled;
    }
    g_count = 0;
    g_hasStart = false;
    char line[256];
    char parts[10][32];
    while (f.ReadLine(line, sizeof(line))) {
        TrimString(line);
        if (line[0] == '\0') {
            continue;
        }
        int n = ExplodeString(line, " ", parts, sizeof(parts), sizeof(parts[]));
        if (StrEqual(parts[0], "start") && n >= 9) {
            for (int k = 0; k < 3; k++) {
                g_startPos[k] = StringToFloat(parts[1 + k]);
                g_startVel[k] = StringToFloat(parts[6 + k]);
            }
            g_startAng[0] = StringToFloat(parts[4]);
            g_startAng[1] = StringToFloat(parts[5]);
            g_startAng[2] = 0.0;
            g_hasStart = true;
        } else if (n >= 5 && g_count < MAX_TICKS) {
            g_buttons[g_count] = StringToInt(parts[0]);
            g_forward[g_count] = StringToFloat(parts[1]);
            g_side[g_count] = StringToFloat(parts[2]);
            g_pitch[g_count] = StringToFloat(parts[3]);
            g_yaw[g_count] = StringToFloat(parts[4]);
            g_count++;
        }
    }
    delete f;

    g_bot = FindBot();
    if (g_bot == 0) {
        PrintToServer("mashup: no living bot");
        return Plugin_Handled;
    }
    if (g_out != null) {
        delete g_out;
    }
    g_out = OpenFile(outPath, "w");
    if (g_out == null) {
        PrintToServer("mashup: cannot write %s", outPath);
        return Plugin_Handled;
    }
    // Knife out: max speed 250, the value mashup assumes.
    FakeClientCommand(g_bot, "use weapon_knife");
    g_cursor = -1;
    PrintToServer("mashup: running %d ticks on client %d", g_count, g_bot);
    return Plugin_Handled;
}

public Action OnPlayerRunCmd(int client, int &buttons, int &impulse, float vel[3], float angles[3],
    int &weapon, int &subtype, int &cmdnum, int &tickcount, int &seed, int mouse[2])
{
    if (client != g_bot || g_out == null) {
        return Plugin_Continue;
    }
    if (g_cursor < 0) {
        if (g_hasStart) {
            TeleportEntity(client, g_startPos, g_startAng, g_startVel);
            angles[0] = g_startAng[0];
            angles[1] = g_startAng[1];
        }
        buttons = 0;
        vel[0] = 0.0;
        vel[1] = 0.0;
        vel[2] = 0.0;
        return Plugin_Changed;
    }
    if (g_cursor >= g_count) {
        return Plugin_Continue;
    }
    buttons = g_buttons[g_cursor];
    vel[0] = g_forward[g_cursor];
    vel[1] = g_side[g_cursor];
    vel[2] = 0.0;
    angles[0] = g_pitch[g_cursor];
    angles[1] = g_yaw[g_cursor];
    angles[2] = 0.0;
    return Plugin_Changed;
}

public void OnPlayerRunCmdPost(int client, int buttons, int impulse, const float vel[3], const float angles[3],
    int weapon, int subtype, int cmdnum, int tickcount, int seed, const int mouse[2])
{
    if (client != g_bot || g_out == null) {
        return;
    }
    if (g_cursor >= g_count) {
        delete g_out;
        g_out = null;
        PrintToServer("mashup: done");
        return;
    }
    float o[3];
    float v[3];
    GetClientAbsOrigin(client, o);
    GetEntPropVector(client, Prop_Data, "m_vecAbsVelocity", v);
    int flags = GetEntityFlags(client);
    g_cursor++;
    g_out.WriteLine("%d %.4f %.4f %.4f %.4f %.4f %.4f %d %d", g_cursor, o[0], o[1], o[2], v[0], v[1], v[2],
        (flags & FL_ONGROUND) ? 1 : 0, (flags & FL_DUCKING) ? 1 : 0);
}
