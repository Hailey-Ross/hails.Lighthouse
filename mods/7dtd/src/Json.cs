using System;
using System.Collections.Generic;
using System.Globalization;
using System.Text;

namespace Lighthouse
{
    /// <summary>
    /// Just enough JSON for heartbeats: a writer, and a small parser for Lighthouse's replies.
    /// Parsed values are Dictionary&lt;string, object&gt;, List&lt;object&gt;, string, double, bool or null.
    /// </summary>
    internal static class Json
    {
        public static string Quote(string s)
        {
            if (s == null) return "null";
            var sb = new StringBuilder(s.Length + 2);
            sb.Append('"');
            foreach (var c in s)
            {
                switch (c)
                {
                    case '"': sb.Append("\\\""); break;
                    case '\\': sb.Append("\\\\"); break;
                    case '\n': sb.Append("\\n"); break;
                    case '\r': sb.Append("\\r"); break;
                    case '\t': sb.Append("\\t"); break;
                    default:
                        if (c < 0x20) sb.AppendFormat("\\u{0:x4}", (int)c);
                        else sb.Append(c);
                        break;
                }
            }
            sb.Append('"');
            return sb.ToString();
        }

        public static object Parse(string text)
        {
            int i = 0;
            var value = ParseValue(text, ref i);
            return value;
        }

        private static void SkipSpace(string s, ref int i)
        {
            while (i < s.Length && char.IsWhiteSpace(s[i])) i++;
        }

        private static object ParseValue(string s, ref int i)
        {
            SkipSpace(s, ref i);
            if (i >= s.Length) throw new FormatException("unexpected end");
            char c = s[i];
            if (c == '{')
            {
                var obj = new Dictionary<string, object>();
                i++;
                SkipSpace(s, ref i);
                if (s[i] == '}') { i++; return obj; }
                while (true)
                {
                    SkipSpace(s, ref i);
                    var key = ParseString(s, ref i);
                    SkipSpace(s, ref i);
                    if (s[i++] != ':') throw new FormatException("expected :");
                    obj[key] = ParseValue(s, ref i);
                    SkipSpace(s, ref i);
                    if (s[i] == ',') { i++; continue; }
                    if (s[i] == '}') { i++; return obj; }
                    throw new FormatException("expected , or }");
                }
            }
            if (c == '[')
            {
                var list = new List<object>();
                i++;
                SkipSpace(s, ref i);
                if (s[i] == ']') { i++; return list; }
                while (true)
                {
                    list.Add(ParseValue(s, ref i));
                    SkipSpace(s, ref i);
                    if (s[i] == ',') { i++; continue; }
                    if (s[i] == ']') { i++; return list; }
                    throw new FormatException("expected , or ]");
                }
            }
            if (c == '"') return ParseString(s, ref i);
            if (s.Substring(i).StartsWith("true")) { i += 4; return true; }
            if (s.Substring(i).StartsWith("false")) { i += 5; return false; }
            if (s.Substring(i).StartsWith("null")) { i += 4; return null; }
            int start = i;
            while (i < s.Length && "+-0123456789.eE".IndexOf(s[i]) >= 0) i++;
            return double.Parse(s.Substring(start, i - start), CultureInfo.InvariantCulture);
        }

        private static string ParseString(string s, ref int i)
        {
            if (s[i] != '"') throw new FormatException("expected string");
            i++;
            var sb = new StringBuilder();
            while (s[i] != '"')
            {
                char c = s[i++];
                if (c != '\\') { sb.Append(c); continue; }
                char e = s[i++];
                switch (e)
                {
                    case 'n': sb.Append('\n'); break;
                    case 'r': sb.Append('\r'); break;
                    case 't': sb.Append('\t'); break;
                    case 'b': sb.Append('\b'); break;
                    case 'f': sb.Append('\f'); break;
                    case 'u': sb.Append((char)Convert.ToInt32(s.Substring(i, 4), 16)); i += 4; break;
                    default: sb.Append(e); break;
                }
            }
            i++;
            return sb.ToString();
        }

        public static string Str(Dictionary<string, object> obj, string key)
        {
            object v;
            return obj != null && obj.TryGetValue(key, out v) ? v as string : null;
        }

        public static double? Num(Dictionary<string, object> obj, string key)
        {
            object v;
            return obj != null && obj.TryGetValue(key, out v) && v is double ? (double?)v : null;
        }
    }
}
