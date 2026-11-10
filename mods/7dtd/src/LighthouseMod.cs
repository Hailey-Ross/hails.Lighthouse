using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Net;
using System.Text;
using System.Threading;
using System.Xml;

namespace Lighthouse
{
    /// <summary>
    /// Server-side mod: every 30 seconds sends Lighthouse a heartbeat (real uptime, players with
    /// platform ids, in-game time, blood moon, new console lines) and carries out the commands
    /// Lighthouse sends back (stop, restart, message, kick, ban) through the server console.
    ///
    /// Settings: Mods/Lighthouse/Config.xml (see Config.example.xml).
    /// Game objects are only touched on the main thread; the network work runs on a background thread.
    /// </summary>
    public class LighthouseMod : IModApi
    {
        private const int IntervalMs = 30000;
        private const int MaxLogLines = 400;

        private static string url;
        private static string secret;
        private static string serverId;
        private static bool sendConsole = true;

        private static readonly long StartedAt =
            new DateTimeOffset(Process.GetCurrentProcess().StartTime.ToUniversalTime()).ToUnixTimeSeconds();

        private static readonly ConcurrentQueue<string> LogLines = new ConcurrentQueue<string>();
        private static readonly ConcurrentQueue<Dictionary<string, object>> Commands =
            new ConcurrentQueue<Dictionary<string, object>>();
        private static volatile bool shuttingDown;
        private static int offlineSent;
        private static int inFlight;
        private static Timer timer;

        public void InitMod(Mod modInstance)
        {
            if (!LoadConfig(modInstance.Path))
            {
                Log.Warning("[Lighthouse] Not configured: copy Config.example.xml to Config.xml in "
                            + modInstance.Path + " and fill it in.");
                return;
            }
            ServicePointManager.SecurityProtocol |= SecurityProtocolType.Tls12;
            if (sendConsole)
            {
                Log.LogCallbacks += OnLog;
            }
            AppDomain.CurrentDomain.ProcessExit += (s, e) => SendOfflineNow();
            timer = new Timer(_ => ThreadManager.AddSingleTaskMainThread("Lighthouse.Heartbeat", Tick), null,
                IntervalMs, IntervalMs);
            Log.Out("[Lighthouse] Reporting to " + url + " as \"" + serverId + "\"");
        }

        private static bool LoadConfig(string modPath)
        {
            var file = Path.Combine(modPath, "Config.xml");
            if (!File.Exists(file)) return false;
            var doc = new XmlDocument();
            doc.Load(file);
            var root = doc.DocumentElement;
            if (root == null) return false;
            url = (root.GetAttribute("url") ?? "").TrimEnd('/');
            secret = root.GetAttribute("secret");
            serverId = root.GetAttribute("serverId");
            sendConsole = root.GetAttribute("console") != "false";
            return url != "" && secret != "" && serverId != "";
        }

        private static void OnLog(string msg, string trace, UnityEngine.LogType type)
        {
            if (string.IsNullOrEmpty(msg) || msg.StartsWith("[Lighthouse]")) return;
            LogLines.Enqueue(msg.Length > 2000 ? msg.Substring(0, 2000) : msg);
            string dropped;
            while (LogLines.Count > MaxLogLines && LogLines.TryDequeue(out dropped)) { }
        }

        // ---------- main thread ----------

        private static void Tick(object _parameter)
        {
            Dictionary<string, object> cmd;
            while (Commands.TryDequeue(out cmd))
            {
                try { Run(cmd); }
                catch (Exception e) { Log.Warning("[Lighthouse] Command failed: " + e.Message); }
            }
            if (Interlocked.CompareExchange(ref inFlight, 1, 0) != 0) return;
            string body;
            try { body = BuildHeartbeat(); }
            catch (Exception e)
            {
                Interlocked.Exchange(ref inFlight, 0);
                Log.Warning("[Lighthouse] Could not read the server status: " + e.Message);
                return;
            }
            ThreadPool.QueueUserWorkItem(_ => Send(body));
        }

        private static string BuildHeartbeat()
        {
            var now = DateTimeOffset.UtcNow.ToUnixTimeSeconds();
            var sb = new StringBuilder(4096);
            sb.Append("{\"v\":1,\"server_id\":").Append(Json.Quote(serverId));
            sb.Append(",\"game\":\"7dtd\",\"sent_at_unix\":").Append(now);
            sb.Append(",\"accepts\":[\"stop\",\"restart\",\"say\",\"kick\",\"ban\"]");
            sb.Append(",\"status\":{\"state\":").Append(shuttingDown ? "\"offline\"" : "\"online\"");
            sb.Append(",\"name\":").Append(Json.Quote(GamePrefs.GetString(EnumGamePrefs.ServerName)));
            sb.Append(",\"players_max\":").Append(GamePrefs.GetInt(EnumGamePrefs.ServerMaxPlayerCount));
            sb.Append(",\"started_at_unix\":").Append(StartedAt);
            sb.Append(",\"observed_at_unix\":").Append(now);

            var world = GameManager.Instance != null ? GameManager.Instance.World : null;
            if (world != null)
            {
                ulong t = world.worldTime;
                int day = GameUtils.WorldTimeToDays(t);
                sb.Append(",\"game_time\":{\"day\":").Append(day)
                  .Append(",\"hour\":").Append(GameUtils.WorldTimeToHours(t))
                  .Append(",\"minute\":").Append(GameUtils.WorldTimeToMinutes(t)).Append('}');
                int frequency = GamePrefs.GetInt(EnumGamePrefs.BloodMoonFrequency);
                int range = GamePrefs.GetInt(EnumGamePrefs.BloodMoonRange);
                if (frequency > 0 && range == 0 && day > 0)
                {
                    int next = (day + frequency - 1) / frequency * frequency;
                    sb.Append(",\"blood_moon_in_days\":").Append(next - day);
                }
            }

            var clients = ConnectionManager.Instance != null ? ConnectionManager.Instance.Clients.List : null;
            int count = 0;
            var players = new StringBuilder();
            if (clients != null)
            {
                foreach (var c in clients)
                {
                    if (c == null || string.IsNullOrEmpty(c.playerName)) continue;
                    if (count++ > 0) players.Append(',');
                    players.Append("{\"name\":").Append(Json.Quote(c.playerName))
                           .Append(",\"ping_ms\":").Append(Math.Max(0, c.ping))
                           .Append(",\"id\":").Append(Json.Quote(c.entityId.ToString(CultureInfo.InvariantCulture)));
                    var ids = new List<string>();
                    if (c.PlatformId != null) ids.Add(Json.Quote(c.PlatformId.CombinedString));
                    if (c.CrossplatformId != null) ids.Add(Json.Quote(c.CrossplatformId.CombinedString));
                    if (ids.Count > 0) players.Append(",\"ids\":[").Append(string.Join(",", ids.ToArray())).Append(']');
                    players.Append('}');
                }
            }
            sb.Append(",\"players_online\":").Append(count);
            if (count > 0) sb.Append(",\"players\":[").Append(players).Append(']');
            sb.Append('}');

            var lines = new List<string>();
            string line;
            while (lines.Count < MaxLogLines && LogLines.TryDequeue(out line)) lines.Add(Json.Quote(line));
            if (lines.Count > 0) sb.Append(",\"log\":[").Append(string.Join(",", lines.ToArray())).Append(']');
            sb.Append('}');
            return sb.ToString();
        }

        private static string Q(object v)
        {
            return "\"" + (v as string ?? "").Replace("\"", " ").Replace("\n", " ").Trim() + "\"";
        }

        /// <summary>Carry out one command from Lighthouse through the server console.</summary>
        private static void Run(Dictionary<string, object> c)
        {
            var action = Json.Str(c, "action");
            string console = null;
            switch (action)
            {
                case "say":
                    console = "say " + Q(Json.Str(c, "message"));
                    break;
                case "kick":
                    console = "kick " + Q(Json.Str(c, "player")) + " " + Q(Json.Str(c, "reason"));
                    break;
                case "ban":
                    var secs = Json.Num(c, "duration_secs");
                    var duration = secs.HasValue ? Math.Max(1, (int)Math.Ceiling(secs.Value / 60)) + " minutes" : "100 years";
                    console = "ban add " + Q(Json.Str(c, "player")) + " " + duration + " " + Q(Json.Str(c, "reason"));
                    break;
                case "stop":
                case "restart":
                    // A restart needs the host (panel auto-restart or a service) to start the server again.
                    shuttingDown = true;
                    SendOfflineNow();
                    console = "shutdown";
                    break;
            }
            if (console == null) return;
            Log.Out("[Lighthouse] Running: " + console);
            SdtdConsole.Instance.ExecuteSync(console, null);
        }

        // ---------- network (background thread) ----------

        private static string Post(string body, int timeoutMs)
        {
            var req = (HttpWebRequest)WebRequest.Create(url + "/v1/heartbeat");
            req.Method = "POST";
            req.ContentType = "application/json";
            req.Headers["Authorization"] = "Bearer " + secret;
            req.Timeout = timeoutMs;
            req.ReadWriteTimeout = timeoutMs;
            var bytes = Encoding.UTF8.GetBytes(body);
            req.ContentLength = bytes.Length;
            using (var s = req.GetRequestStream()) s.Write(bytes, 0, bytes.Length);
            using (var resp = (HttpWebResponse)req.GetResponse())
            using (var reader = new StreamReader(resp.GetResponseStream(), Encoding.UTF8))
            {
                return reader.ReadToEnd();
            }
        }

        private static void Send(string body)
        {
            try
            {
                var reply = Json.Parse(Post(body, 10000)) as Dictionary<string, object>;
                object list;
                if (reply != null && reply.TryGetValue("commands", out list) && list is List<object>)
                {
                    foreach (var c in (List<object>)list)
                    {
                        var cmd = c as Dictionary<string, object>;
                        if (cmd != null) Commands.Enqueue(cmd);
                    }
                    if (((List<object>)list).Count > 0)
                    {
                        ThreadManager.AddSingleTaskMainThread("Lighthouse.Commands", Tick);
                    }
                }
            }
            catch (WebException e)
            {
                var resp = e.Response as HttpWebResponse;
                Log.Warning("[Lighthouse] Heartbeat failed: " + (resp != null ? ((int)resp.StatusCode).ToString() : e.Message));
            }
            catch (Exception e)
            {
                Log.Warning("[Lighthouse] Heartbeat failed: " + e.Message);
            }
            finally
            {
                Interlocked.Exchange(ref inFlight, 0);
            }
        }

        /// <summary>Tell Lighthouse right away that the server is going down (best effort).</summary>
        private static void SendOfflineNow()
        {
            // Once only: a stop from Lighthouse sends it, then the process-exit hook would again.
            if (url == null || Interlocked.Exchange(ref offlineSent, 1) == 1) return;
            try
            {
                var now = DateTimeOffset.UtcNow.ToUnixTimeSeconds();
                Post("{\"v\":1,\"server_id\":" + Json.Quote(serverId) + ",\"game\":\"7dtd\",\"sent_at_unix\":" + now
                     + ",\"status\":{\"state\":\"offline\",\"observed_at_unix\":" + now + "}"
                     + ",\"events\":[{\"kind\":\"shutdown\",\"author\":\"server\"}]}", 3000);
            }
            catch (Exception)
            {
                // Lighthouse notices the missing heartbeats instead.
            }
        }
    }
}
