using System;
using System.Collections;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Drawing.Text;
using System.Globalization;
using System.IO;
using System.Text;
using System.Web.Script.Serialization;
using System.Windows.Forms;

namespace Wideband
{
    sealed class TuneTable
    {
        public string Id = "";
        public string Name = "";
        public string Domain = "";
        public string Level = "beginner";
        public string Kind = "map";
        public string Unit = "";
        public string XName = "";
        public string YName = "";
        public double[] X = new double[0];
        public double[] Y = new double[0];
        public double[][] Values = new double[0][];
        public double[][] Base = new double[0][];
        public int Decimals = -1;
        public XdfBinding Binding;

        public int LevelRank()
        {
            if (Level == "advanced") return 2;
            if (Level == "intermediate") return 1;
            return 0;
        }

        public override string ToString()
        {
            return Name;
        }
    }

    sealed class TuneDocument
    {
        public readonly List<TuneTable> Tables = new List<TuneTable>();
        public string Note = "";
        public bool Persist = true;
        public event EventHandler Changed;

        public TuneTable Find(string id)
        {
            foreach (TuneTable table in Tables)
                if (table.Id == id) return table;
            return null;
        }

        public void Notify()
        {
            if (Changed != null) Changed(this, EventArgs.Empty);
        }

        public static TuneDocument LoadOrCreate(string path)
        {
            if (File.Exists(path))
            {
                try { return Read(File.ReadAllText(path)); }
                catch (Exception) { }
            }
            TuneDocument created = Sample();
            try { created.Write(path); }
            catch (Exception) { }
            return created;
        }

        public void Write(string path)
        {
            if (!Persist)
                throw new InvalidOperationException("This definition is not written to the sample file.");
            var serializer = new JavaScriptSerializer();
            var tables = new List<Dictionary<string, object>>();
            foreach (TuneTable table in Tables)
            {
                var item = new Dictionary<string, object>();
                item["id"] = table.Id;
                item["name"] = table.Name;
                item["domain"] = table.Domain;
                item["level"] = table.Level;
                item["kind"] = table.Kind;
                item["unit"] = table.Unit;
                item["xName"] = table.XName;
                item["yName"] = table.YName;
                item["x"] = table.X;
                item["y"] = table.Y;
                item["values"] = table.Values;
                tables.Add(item);
            }
            var root = new Dictionary<string, object>();
            root["version"] = 1;
            root["note"] = Note;
            root["tables"] = tables;
            File.WriteAllText(path, serializer.Serialize(root));
        }

        static TuneDocument Read(string json)
        {
            var serializer = new JavaScriptSerializer();
            var root = serializer.Deserialize<Dictionary<string, object>>(json);
            var document = new TuneDocument();
            object note;
            if (root.TryGetValue("note", out note) && note != null) document.Note = note.ToString();
            var tables = root["tables"] as ArrayList;
            foreach (object itemObj in tables)
            {
                var item = (Dictionary<string, object>)itemObj;
                var table = new TuneTable();
                table.Id = item["id"].ToString();
                table.Name = item["name"].ToString();
                table.Domain = item["domain"].ToString();
                table.Level = item["level"].ToString();
                table.Kind = item["kind"].ToString();
                table.Unit = item["unit"].ToString();
                table.XName = item["xName"].ToString();
                table.YName = item["yName"].ToString();
                table.X = Numbers(item["x"]);
                table.Y = Numbers(item["y"]);
                table.Values = Grid(item["values"]);
                table.Base = Clone(table.Values);
                document.Tables.Add(table);
            }
            return document;
        }

        public static TuneDocument Sample()
        {
            var document = new TuneDocument();
            document.Note = "Sample calibration. Not read from an ECU.";
            document.Tables.Add(Map("ve", "Volumetric efficiency", "Airflow", "intermediate", "%", "RPM", "MAP kPa",
                new double[] { 1000, 2000, 3000, 4000, 5000, 6000 },
                new double[] { 20, 40, 60, 80, 100 },
                new double[][] {
                    new double[] { 45, 50, 55, 58, 60, 62 },
                    new double[] { 55, 62, 70, 74, 76, 78 },
                    new double[] { 60, 70, 78, 82, 84, 85 },
                    new double[] { 62, 72, 80, 86, 88, 89 },
                    new double[] { 64, 74, 82, 88, 90, 91 }
                }));
            document.Tables.Add(Curve("lambda", "Target lambda", "Fuel", "beginner", "lambda", "Load %",
                new double[] { 20, 40, 60, 80, 100 },
                new double[] { 1.00, 0.98, 0.90, 0.86, 0.82 }));
            document.Tables.Add(Curve("deadtime", "Injector dead time", "Fuel", "advanced", "ms", "Volts",
                new double[] { 10, 12, 14, 16 },
                new double[] { 1.20, 0.90, 0.70, 0.60 }));
            document.Tables.Add(Curve("warmup", "Warm-up enrichment", "Fuel", "intermediate", "%", "Coolant C",
                new double[] { -10, 0, 20, 40, 60, 80 },
                new double[] { 140, 125, 110, 104, 100, 100 }));
            document.Tables.Add(Map("spark", "Base spark", "Ignition", "intermediate", "deg", "RPM", "MAP kPa",
                new double[] { 1000, 2000, 3000, 4000, 5000, 6000 },
                new double[] { 20, 40, 60, 80, 100 },
                new double[][] {
                    new double[] { 12, 16, 20, 24, 26, 28 },
                    new double[] { 10, 14, 18, 22, 24, 26 },
                    new double[] { 8, 12, 16, 18, 20, 22 },
                    new double[] { 6, 10, 12, 14, 16, 16 },
                    new double[] { 4, 6, 8, 10, 12, 12 }
                }));
            document.Tables.Add(Curve("iat", "Intake temp spark trim", "Ignition", "intermediate", "deg", "IAT C",
                new double[] { 0, 20, 40, 60 },
                new double[] { 2, 0, -2, -4 }));
            document.Tables.Add(Curve("boost", "Boost target", "Induction", "beginner", "kPa", "RPM",
                new double[] { 2000, 3000, 4000, 5000, 6000 },
                new double[] { 110, 130, 150, 160, 160 }));
            document.Tables.Add(Curve("wastegate", "Wastegate duty", "Induction", "intermediate", "%", "RPM",
                new double[] { 2000, 3000, 4000, 5000, 6000 },
                new double[] { 20, 35, 50, 60, 65 }));
            document.Tables.Add(Curve("pedal", "Pedal to throttle", "Throttle", "beginner", "%", "Pedal %",
                new double[] { 0, 25, 50, 75, 100 },
                new double[] { 0, 20, 45, 75, 100 }));
            document.Tables.Add(Curve("idle", "Idle target", "Throttle", "beginner", "RPM", "Coolant C",
                new double[] { -10, 0, 20, 40, 80 },
                new double[] { 1200, 1100, 950, 800, 750 }));
            document.Tables.Add(Scalar("launch", "Launch limit", "Motorsport", "advanced", "RPM", 3500));
            document.Tables.Add(Scalar("flex", "Flex blend", "Motorsport", "advanced", "% ethanol", 0));
            document.Tables.Add(Scalar("rev", "Rev limit", "Motorsport", "advanced", "RPM", 6500));
            return document;
        }

        static TuneTable Scalar(string id, string name, string domain, string level, string unit, double value)
        {
            return Map(id, name, domain, level, unit, "Value", "Value", new double[] { 0 }, new double[] { 0 }, new double[][] { new double[] { value } });
        }

        static TuneTable Curve(string id, string name, string domain, string level, string unit, string xName, double[] x, double[] values)
        {
            var table = Map(id, name, domain, level, unit, xName, "Output", x, new double[] { 0 }, new double[][] { values });
            table.Kind = "curve";
            return table;
        }

        static TuneTable Map(string id, string name, string domain, string level, string unit, string xName, string yName, double[] x, double[] y, double[][] values)
        {
            var table = new TuneTable();
            table.Id = id;
            table.Name = name;
            table.Domain = domain;
            table.Level = level;
            table.Kind = y.Length > 1 ? "map" : "scalar";
            if (y.Length > 1) table.Kind = "map";
            table.Unit = unit;
            table.XName = xName;
            table.YName = yName;
            table.X = x;
            table.Y = y;
            table.Values = Clone(values);
            table.Base = Clone(values);
            return table;
        }

        public static double[][] Clone(double[][] source)
        {
            var copy = new double[source.Length][];
            for (int y = 0; y < source.Length; y++)
            {
                copy[y] = new double[source[y].Length];
                for (int x = 0; x < source[y].Length; x++) copy[y][x] = source[y][x];
            }
            return copy;
        }

        static double[] Numbers(object value)
        {
            var list = (ArrayList)value;
            var numbers = new double[list.Count];
            for (int i = 0; i < list.Count; i++) numbers[i] = Convert.ToDouble(list[i], CultureInfo.InvariantCulture);
            return numbers;
        }

        static double[][] Grid(object value)
        {
            var rows = (ArrayList)value;
            var grid = new double[rows.Count][];
            for (int y = 0; y < rows.Count; y++) grid[y] = Numbers(rows[y]);
            return grid;
        }
    }

    static class TableMath
    {
        public static void Add(TuneTable table, IList selected, double amount)
        {
            Each(table, selected, delegate (int y, int x) { table.Values[y][x] += amount; });
        }

        public static void Scale(TuneTable table, IList selected, double percent)
        {
            double factor = 1.0 + percent / 100.0;
            Each(table, selected, delegate (int y, int x) { table.Values[y][x] *= factor; });
        }

        public static void Smooth(TuneTable table, IList selected)
        {
            double[][] next = TuneDocument.Clone(table.Values);
            Each(table, selected, delegate (int y, int x)
            {
                double sum = table.Values[y][x];
                int count = 1;
                if (y > 0) { sum += table.Values[y - 1][x]; count++; }
                if (y + 1 < table.Values.Length) { sum += table.Values[y + 1][x]; count++; }
                if (x > 0) { sum += table.Values[y][x - 1]; count++; }
                if (x + 1 < table.Values[y].Length) { sum += table.Values[y][x + 1]; count++; }
                next[y][x] = sum / count;
            });
            table.Values = next;
        }

        public static void Set(TuneTable table, IList selected, double value)
        {
            Each(table, selected, delegate (int y, int x) { table.Values[y][x] = value; });
        }

        public static void Interpolate(TuneTable table, IList selected)
        {
            int y0, x0, y1, x1;
            if (!Bounds(table, selected, out y0, out x0, out y1, out x1)) return;
            double c00 = table.Values[y0][x0];
            double c01 = table.Values[y0][x1];
            double c10 = table.Values[y1][x0];
            double c11 = table.Values[y1][x1];
            for (int y = y0; y <= y1; y++)
            {
                double v = y1 == y0 ? 0 : (double)(y - y0) / (y1 - y0);
                for (int x = x0; x <= x1; x++)
                {
                    double u = x1 == x0 ? 0 : (double)(x - x0) / (x1 - x0);
                    double top = c00 + (c01 - c00) * u;
                    double bottom = c10 + (c11 - c10) * u;
                    table.Values[y][x] = top + (bottom - top) * v;
                }
            }
        }

        static void Each(TuneTable table, IList selected, Action<int, int> action)
        {
            if (selected == null || selected.Count == 0)
            {
                for (int y = 0; y < table.Values.Length; y++)
                    for (int x = 0; x < table.Values[y].Length; x++) action(y, x);
                return;
            }
            foreach (DataGridViewCell cell in selected)
                if (cell.RowIndex >= 0 && cell.ColumnIndex >= 0) action(cell.RowIndex, cell.ColumnIndex);
        }

        static bool Bounds(TuneTable table, IList selected, out int y0, out int x0, out int y1, out int x1)
        {
            y0 = 0; x0 = 0; y1 = table.Values.Length - 1; x1 = table.Values[0].Length - 1;
            if (selected == null || selected.Count == 0) return true;
            bool any = false;
            foreach (DataGridViewCell cell in selected)
            {
                if (cell.RowIndex < 0 || cell.ColumnIndex < 0) continue;
                if (!any) { y0 = y1 = cell.RowIndex; x0 = x1 = cell.ColumnIndex; any = true; }
                else
                {
                    if (cell.RowIndex < y0) y0 = cell.RowIndex;
                    if (cell.RowIndex > y1) y1 = cell.RowIndex;
                    if (cell.ColumnIndex < x0) x0 = cell.ColumnIndex;
                    if (cell.ColumnIndex > x1) x1 = cell.ColumnIndex;
                }
            }
            return any;
        }

        public static int Nearest(double[] axis, double value)
        {
            int best = 0;
            double distance = Math.Abs(axis[0] - value);
            for (int i = 1; i < axis.Length; i++)
            {
                double next = Math.Abs(axis[i] - value);
                if (next < distance) { distance = next; best = i; }
            }
            return best;
        }
    }

    static class Theme
    {
        public static readonly Color Background = Color.FromArgb(240, 242, 239);
        public static readonly Color Panel = Color.FromArgb(247, 248, 246);
        public static readonly Color SlateHeader = Color.FromArgb(23, 33, 28);
        public static readonly Color Ink = Color.FromArgb(23, 33, 28);
        public static readonly Color Muted = Color.FromArgb(75, 88, 82);
        public static readonly Color Border = Color.FromArgb(195, 202, 196);
        public static readonly Color DarkBorder = Color.FromArgb(160, 170, 162);
        public static readonly Color Accent = Color.FromArgb(12, 110, 74);
        public static readonly Color AccentTint = Color.FromArgb(229, 242, 235);
        public static readonly Color TextLight = Color.FromArgb(244, 250, 247);
        public static readonly Color ValidGreen = Color.FromArgb(214, 236, 214);
        public static readonly Color WarningAmber = Color.FromArgb(244, 228, 196);
        public static readonly Color ErrorCoral = Color.FromArgb(244, 214, 214);

        public static Color ContrastText(Color bg)
        {
            double lum = (0.299 * bg.R + 0.587 * bg.G + 0.114 * bg.B);
            return lum < 135 ? TextLight : Ink;
        }
    }

    static class HeatColor
    {
        public static Color Blend(double unit)
        {
            return Mix(unit, Color.FromArgb(186, 214, 236), Color.FromArgb(196, 224, 186), Color.FromArgb(236, 186, 176));
        }

        public static Color Deep(double unit)
        {
            return Mix(unit, Color.FromArgb(48, 96, 160), Color.FromArgb(36, 140, 88), Color.FromArgb(176, 64, 58));
        }

        static Color Mix(double unit, Color low, Color mid, Color high)
        {
            if (unit < 0) unit = 0;
            if (unit > 1) unit = 1;
            if (unit < 0.5) return Lerp(low, mid, unit / 0.5);
            return Lerp(mid, high, (unit - 0.5) / 0.5);
        }

        static Color Lerp(Color a, Color b, double unit)
        {
            return Color.FromArgb(
                a.R + (int)((b.R - a.R) * unit),
                a.G + (int)((b.G - a.G) * unit),
                a.B + (int)((b.B - a.B) * unit));
        }
    }

    static class CellDelta
    {
        public static bool Changed(double value, double basis)
        {
            return Math.Abs(value - basis) > 0.0001;
        }

        public static string Display(TuneTable table, double value, double basis)
        {
            string text = FormatNumber(table, value);
            double delta = value - basis;
            if (!Changed(value, basis)) return text;
            string sign = delta > 0 ? "+" : "";
            return text + " (" + sign + FormatNumber(table, delta) + ")";
        }

        public static string FormatNumber(TuneTable table, double value)
        {
            if (table == null) return value.ToString("0.###", CultureInfo.InvariantCulture);
            if (table.Decimals == 0) return Math.Round(value).ToString("0", CultureInfo.InvariantCulture);
            if (table.Decimals > 0)
            {
                int places = table.Decimals;
                if (places > 8) places = 8;
                return value.ToString("0." + new string('0', places), CultureInfo.InvariantCulture);
            }
            return value.ToString("0.###", CultureInfo.InvariantCulture);
        }
    }

    static class CellCommand
    {
        public static void Run(TuneTable table, IList selected, string text, double amount)
        {
            if (table == null) throw new ArgumentNullException("table");
            if (text == null) text = "";
            text = text.Trim();
            if (text.Length == 0)
            {
                TableMath.Add(table, selected, amount);
                return;
            }
            if (text[text.Length - 1] == '%')
            {
                TableMath.Scale(table, selected, ParseNumber(text.Substring(0, text.Length - 1)));
                return;
            }
            if (text[0] == '=')
            {
                TableMath.Set(table, selected, ParseNumber(text.Substring(1)));
                return;
            }
            TableMath.Add(table, selected, ParseNumber(text));
        }

        static double ParseNumber(string text)
        {
            if (text == null) text = "";
            text = text.Trim();
            double value;
            if (!double.TryParse(text, NumberStyles.Float, CultureInfo.InvariantCulture, out value))
                throw new FormatException("The command must be +5%, -2%, =value, or a signed number.");
            return value;
        }
    }

    static class CellPaint
    {
        public static string Apply(DataGridViewCellStyle style, TuneTable table, double value, double basis, double unit, bool compare, Font bold, Font regular)
        {
            bool changed = CellDelta.Changed(value, basis);
            if (bold != null && regular != null) style.Font = changed ? bold : regular;
            if (compare)
            {
                double delta = value - basis;
                if (delta > 0.0001) style.BackColor = Theme.ValidGreen;
                else if (delta < -0.0001) style.BackColor = Theme.ErrorCoral;
                else style.BackColor = Theme.Panel;
            }
            else style.BackColor = HeatColor.Blend(unit);
            style.ForeColor = Theme.ContrastText(style.BackColor);
            return CellDelta.Display(table, value, basis);
        }
    }

    sealed class SwitchPoint
    {
        public TuneTable Table;
        public int Y;
        public int X;
    }

    sealed class LogTable
    {
        public string[] Headers = new string[0];
        public List<double[]> Rows = new List<double[]>();

        public int Index(string name)
        {
            for (int i = 0; i < Headers.Length; i++)
                if (string.Compare(Headers[i], name, StringComparison.OrdinalIgnoreCase) == 0) return i;
            return -1;
        }

        public static LogTable Parse(string text)
        {
            var log = new LogTable();
            string[] lines = text.Replace("\r", "").Split('\n');
            int first = 0;
            while (first < lines.Length && lines[first].Trim().Length == 0) first++;
            if (first >= lines.Length) return log;
            log.Headers = Split(lines[first]);
            for (int i = first + 1; i < lines.Length; i++)
            {
                if (lines[i].Trim().Length == 0) continue;
                string[] parts = Split(lines[i]);
                var row = new double[log.Headers.Length];
                for (int c = 0; c < row.Length; c++)
                {
                    double value;
                    string token = c < parts.Length ? parts[c] : "0";
                    if (!double.TryParse(token, NumberStyles.Float, CultureInfo.InvariantCulture, out value)) value = 0;
                    row[c] = value;
                }
                log.Rows.Add(row);
            }
            return log;
        }

        static string[] Split(string line)
        {
            string[] parts = line.Split(',');
            for (int i = 0; i < parts.Length; i++) parts[i] = parts[i].Trim();
            return parts;
        }
    }

    static class ChannelMath
    {
        public static double Eval(string expression, Dictionary<string, double> variables)
        {
            int index = 0;
            double value = ParseAdd(expression, variables, ref index);
            Skip(expression, ref index);
            if (index != expression.Length) throw new FormatException("The channel formula has an unexpected character.");
            return value;
        }

        static double ParseAdd(string expression, Dictionary<string, double> variables, ref int index)
        {
            double value = ParseMul(expression, variables, ref index);
            while (true)
            {
                Skip(expression, ref index);
                if (index >= expression.Length) return value;
                char op = expression[index];
                if (op != '+' && op != '-') return value;
                index++;
                double right = ParseMul(expression, variables, ref index);
                value = op == '+' ? value + right : value - right;
            }
        }

        static double ParseMul(string expression, Dictionary<string, double> variables, ref int index)
        {
            double value = ParseUnary(expression, variables, ref index);
            while (true)
            {
                Skip(expression, ref index);
                if (index >= expression.Length) return value;
                char op = expression[index];
                if (op != '*' && op != '/') return value;
                index++;
                double right = ParseUnary(expression, variables, ref index);
                value = op == '*' ? value * right : value / right;
            }
        }

        static double ParseUnary(string expression, Dictionary<string, double> variables, ref int index)
        {
            Skip(expression, ref index);
            if (index < expression.Length && expression[index] == '-')
            {
                index++;
                return -ParsePrimary(expression, variables, ref index);
            }
            return ParsePrimary(expression, variables, ref index);
        }

        static double ParsePrimary(string expression, Dictionary<string, double> variables, ref int index)
        {
            Skip(expression, ref index);
            if (index >= expression.Length) throw new FormatException("The channel formula ended early.");
            if (expression[index] == '(')
            {
                index++;
                double value = ParseAdd(expression, variables, ref index);
                Skip(expression, ref index);
                if (index >= expression.Length || expression[index] != ')') throw new FormatException("A parenthesis is missing.");
                index++;
                return value;
            }
            int start = index;
            if (char.IsDigit(expression[index]) || expression[index] == '.')
            {
                while (index < expression.Length && (char.IsDigit(expression[index]) || expression[index] == '.')) index++;
                return double.Parse(expression.Substring(start, index - start), CultureInfo.InvariantCulture);
            }
            while (index < expression.Length && (char.IsLetterOrDigit(expression[index]) || expression[index] == '_')) index++;
            string name = expression.Substring(start, index - start);
            double found;
            if (!variables.TryGetValue(name.ToLowerInvariant(), out found))
                throw new FormatException("Unknown channel " + name + ".");
            return found;
        }

        static void Skip(string expression, ref int index)
        {
            while (index < expression.Length && char.IsWhiteSpace(expression[index])) index++;
        }
    }

    sealed class CalibratePanel : UserControl
    {
        TuneDocument document;
        readonly string savePath;
        readonly TreeView tree;
        readonly ComboBox level;
        readonly TuneGrid grid;
        readonly SurfaceMesh mesh;
        readonly NumericUpDown amount;
        readonly NumericUpDown traceX;
        readonly NumericUpDown traceY;
        readonly CheckBox compare;
        readonly Label status;
        readonly Label note;
        readonly Label checksumMark;
        readonly TextBox searchBox;
        readonly TextBox commandBox;
        readonly TextBox switchSearch;
        readonly Button tableViewButton;
        readonly Button switchViewButton;
        readonly Panel switchList;
        readonly Font boldFont;
        readonly Font monoFont;
        readonly Label telemReadout;
        readonly SplitContainer outerSplit;
        readonly SplitContainer innerSplit;
        TuneTable current;
        MemoryImage rom;
        long romOrigin;
        string romNote = "";
        string romPath = "";
        XdfDocument definition;
        bool romBacked;
        bool showSwitches;
        bool fillingSwitches;
        string parameterFilter = "";
        string switchFilter = "";
        double heatMin;
        double heatSpan = 1;

        public CalibratePanel(TuneDocument document, string savePath)
        {
            this.document = document;
            this.savePath = savePath;
            Dock = DockStyle.Fill;
            Size = new Size(1000, 680);
            BackColor = Theme.Panel;

            var commandStrip = new Panel();
            commandStrip.Dock = DockStyle.Top;
            commandStrip.Height = 73;
            commandStrip.BackColor = Theme.Background;
            commandStrip.Padding = new Padding(4, 2, 4, 2);

            var topRow = new FlowLayoutPanel();
            topRow.Dock = DockStyle.Top;
            topRow.Height = 34;
            topRow.WrapContents = false;
            topRow.BackColor = Theme.Background;

            var divider = new Panel();
            divider.Dock = DockStyle.Top;
            divider.Height = 1;
            divider.BackColor = Color.FromArgb(48, 54, 61);

            var bottomRow = new FlowLayoutPanel();
            bottomRow.Dock = DockStyle.Bottom;
            bottomRow.Height = 34;
            bottomRow.WrapContents = false;
            bottomRow.BackColor = Theme.Background;

            topRow.Controls.Add(MakeButton("Open ROM", OpenRomClicked));
            topRow.Controls.Add(MakeButton("Open XDF", OpenXdfClicked));
            topRow.Controls.Add(MakeButton("Save ROM", SaveRomClicked));

            checksumMark = new Label();
            checksumMark.AutoSize = true;
            checksumMark.TextAlign = ContentAlignment.MiddleCenter;
            checksumMark.Padding = new Padding(8, 4, 8, 4);
            checksumMark.Margin = new Padding(4, 2, 4, 2);
            checksumMark.AccessibleName = "Checksum indicator";
            checksumMark.AccessibleRole = AccessibleRole.StaticText;
            checksumMark.Font = BadgeFont();
            SetMark("");
            topRow.Controls.Add(checksumMark);

            topRow.Controls.Add(MakeSeparator());

            tableViewButton = MakeButton("Table", delegate { ShowTableView(); });
            switchViewButton = MakeButton("Switches", delegate { ShowSwitchView(); });
            topRow.Controls.Add(tableViewButton);
            topRow.Controls.Add(switchViewButton);
            PaintViewButtons();

            compare = new CheckBox();
            compare.Text = "Compare";
            compare.AutoSize = true;
            compare.Margin = new Padding(6, 6, 4, 2);
            compare.CheckedChanged += delegate { grid.Invalidate(); };
            topRow.Controls.Add(compare);

            topRow.Controls.Add(MakeSeparator());

            var traceLabel = LabelFor("Trace");
            traceX = NumberBox(3000, 0, 20000, 0);
            traceX.AccessibleName = "Trace RPM";
            traceX.Width = 72;
            traceY = NumberBox(60, 0, 500, 0);
            traceY.AccessibleName = "Trace load";
            traceY.Width = 60;
            topRow.Controls.Add(traceLabel);
            topRow.Controls.Add(traceX);
            topRow.Controls.Add(traceY);

            commandStrip.Controls.Add(bottomRow);
            commandStrip.Controls.Add(divider);
            commandStrip.Controls.Add(topRow);

            bottomRow.Controls.Add(LabelFor("Safety"));
            level = new ComboBox();
            level.DropDownStyle = ComboBoxStyle.DropDownList;
            level.Width = 110;
            level.Items.AddRange(new object[] { "Beginner", "Intermediate", "Advanced" });
            level.SelectedIndex = 1;
            level.AccessibleName = "Safety level";
            level.SelectedIndexChanged += delegate { FillList(); };
            bottomRow.Controls.Add(level);

            bottomRow.Controls.Add(MakeSeparator());

            amount = NumberBox(10, -1000, 1000, 0);
            amount.AccessibleName = "Amount";
            amount.Width = 64;
            bottomRow.Controls.Add(amount);
            bottomRow.Controls.Add(MakeButton("Add", AddClicked));
            bottomRow.Controls.Add(MakeButton("Scale %", ScaleClicked));
            bottomRow.Controls.Add(MakeButton("Smooth", delegate { Run(delegate { TableMath.Smooth(current, grid.SelectedCells); }, "Smoothed the selected cells."); }));
            bottomRow.Controls.Add(MakeButton("Interpolate", delegate { InterpolateSelection(); }));

            bottomRow.Controls.Add(MakeSeparator());

            bottomRow.Controls.Add(LabelFor("Cmd"));
            commandBox = new TextBox();
            commandBox.Width = 90;
            commandBox.AccessibleName = "Cell command";
            commandBox.KeyDown += CommandKeyDown;
            bottomRow.Controls.Add(commandBox);
            bottomRow.Controls.Add(MakeButton("Apply", ApplyClicked));

            var statusBar = new Panel();
            statusBar.Dock = DockStyle.Bottom;
            statusBar.Height = 28;
            statusBar.BackColor = Theme.Panel;
            statusBar.Padding = new Padding(8, 4, 8, 4);

            status = new Label();
            status.Dock = DockStyle.Left;
            status.AutoSize = true;
            status.Font = new Font(Font, FontStyle.Bold);
            status.ForeColor = Theme.Ink;
            status.Text = "Choose a table.";

            note = new Label();
            note.Dock = DockStyle.Fill;
            note.TextAlign = ContentAlignment.MiddleLeft;
            note.Padding = new Padding(12, 0, 0, 0);
            note.ForeColor = Theme.Muted;
            note.Text = document.Note;

            statusBar.Controls.Add(note);
            statusBar.Controls.Add(status);

            var findLabel = new Label();
            findLabel.Text = "Find";
            findLabel.Dock = DockStyle.Top;
            findLabel.Height = 22;
            findLabel.ForeColor = Theme.Muted;

            searchBox = new TextBox();
            searchBox.Dock = DockStyle.Top;
            searchBox.AccessibleName = "Parameter search";
            searchBox.TextChanged += delegate
            {
                parameterFilter = searchBox.Text.Trim();
                FillList();
            };

            tree = new TreeView();
            tree.Dock = DockStyle.Fill;
            tree.BorderStyle = BorderStyle.None;
            tree.HideSelection = false;
            tree.ShowPlusMinus = true;
            tree.ShowRootLines = true;
            tree.ItemHeight = 30;
            tree.BackColor = Theme.Panel;
            tree.AccessibleName = "Parameter tree";

            var sidebar = new Panel();
            sidebar.Dock = DockStyle.Fill;
            sidebar.BackColor = Theme.Panel;
            sidebar.Controls.Add(tree);
            sidebar.Controls.Add(searchBox);
            sidebar.Controls.Add(findLabel);

            mesh = new SurfaceMesh();
            mesh.Dock = DockStyle.Fill;

            monoFont = new Font("Consolas", 9.5f, FontStyle.Regular);
            boldFont = new Font("Consolas", 9.5f, FontStyle.Bold);

            grid = new TuneGrid();
            grid.Dock = DockStyle.Fill;
            grid.AllowUserToAddRows = false;
            grid.AllowUserToDeleteRows = false;
            grid.RowHeadersWidth = 72;
            grid.BackgroundColor = Theme.Panel;
            grid.GridColor = Theme.Border;
            grid.BorderStyle = BorderStyle.None;
            grid.SelectionMode = DataGridViewSelectionMode.CellSelect;
            grid.CellEndEdit += CellEdited;
            grid.CellFormatting += FormatCell;
            grid.Nudge += Nudge;
            grid.Gesture += GridGesture;
            grid.DefaultCellStyle.BackColor = Theme.Panel;
            grid.DefaultCellStyle.ForeColor = Theme.Ink;
            grid.DefaultCellStyle.SelectionBackColor = Theme.Accent;
            grid.DefaultCellStyle.SelectionForeColor = Color.White;
            grid.DefaultCellStyle.Font = monoFont;
            grid.DefaultCellStyle.Alignment = DataGridViewContentAlignment.MiddleRight;
            grid.ColumnHeadersDefaultCellStyle.BackColor = Theme.SlateHeader;
            grid.ColumnHeadersDefaultCellStyle.ForeColor = Color.White;
            grid.ColumnHeadersDefaultCellStyle.Font = boldFont;
            grid.ColumnHeadersDefaultCellStyle.Alignment = DataGridViewContentAlignment.MiddleRight;
            grid.RowHeadersDefaultCellStyle.BackColor = Theme.SlateHeader;
            grid.RowHeadersDefaultCellStyle.ForeColor = Color.White;
            grid.RowHeadersDefaultCellStyle.Font = boldFont;
            grid.EnableHeadersVisualStyles = false;

            traceX.ValueChanged += delegate { Trace(); };
            traceY.ValueChanged += delegate { Trace(); };
            grid.CurrentCellChanged += delegate
            {
                if (grid.CurrentCell != null)
                    mesh.SetActiveCell(grid.CurrentCell.RowIndex, grid.CurrentCell.ColumnIndex);
            };

            switchSearch = new TextBox();
            switchSearch.Dock = DockStyle.Top;
            switchSearch.AccessibleName = "Switch search";
            switchSearch.TextChanged += delegate
            {
                switchFilter = switchSearch.Text.Trim();
                RebuildSwitches();
            };
            switchList = new Panel();
            switchList.Dock = DockStyle.Fill;
            switchList.AutoScroll = true;
            switchList.BackColor = Theme.Panel;

            var switchBoard = new Panel();
            switchBoard.Dock = DockStyle.Fill;
            switchBoard.Visible = false;
            switchBoard.BackColor = Theme.Panel;
            switchBoard.AccessibleName = "Switch board";
            switchBoard.Controls.Add(switchList);
            switchBoard.Controls.Add(switchSearch);

            var centerWorkspace = new Panel();
            centerWorkspace.Dock = DockStyle.Fill;
            centerWorkspace.BackColor = Theme.Panel;
            centerWorkspace.Controls.Add(grid);
            centerWorkspace.Controls.Add(switchBoard);

            var telemCard = new Panel();
            telemCard.Dock = DockStyle.Bottom;
            telemCard.Height = 90;
            telemCard.BackColor = Color.FromArgb(17, 21, 28);
            telemCard.Padding = new Padding(10, 8, 10, 8);
            telemCard.Paint += delegate(object s, PaintEventArgs pe)
            {
                using (var pen = new Pen(Color.FromArgb(48, 54, 61), 1f))
                {
                    pe.Graphics.DrawRectangle(pen, 0, 0, telemCard.Width - 1, telemCard.Height - 1);
                }
            };

            var telemHeader = new Label();
            telemHeader.Dock = DockStyle.Top;
            telemHeader.Height = 20;
            telemHeader.Text = "LIVE CELL TELEMETRY TRACKING";
            telemHeader.Font = new Font("Segoe UI", 8.25f, FontStyle.Bold);
            telemHeader.ForeColor = Color.FromArgb(140, 150, 165);

            telemReadout = new Label();
            telemReadout.Dock = DockStyle.Fill;
            telemReadout.Font = new Font("Consolas", 9.5f, FontStyle.Bold);
            telemReadout.ForeColor = Color.FromArgb(0, 229, 255);
            telemReadout.Text = "RPM: 3000   MAP: 60.0 kPa\r\nTracked Cell: Row 0, Col 0";

            telemCard.Controls.Add(telemReadout);
            telemCard.Controls.Add(telemHeader);

            innerSplit = new SplitContainer();
            innerSplit.Size = new Size(740, 580);
            innerSplit.Dock = DockStyle.Fill;
            innerSplit.Orientation = Orientation.Vertical;
            innerSplit.SplitterWidth = 5;
            innerSplit.BackColor = Theme.Border;
            innerSplit.Panel1MinSize = 200;
            innerSplit.Panel2MinSize = 240;
            innerSplit.Panel1.Controls.Add(centerWorkspace);
            innerSplit.Panel2.Controls.Add(mesh);
            innerSplit.Panel2.Controls.Add(telemCard);
            try
            {
                if (innerSplit.Width >= 500) innerSplit.SplitterDistance = Math.Max(200, innerSplit.Width - 300);
            }
            catch { }

            outerSplit = new SplitContainer();
            outerSplit.Size = new Size(1000, 580);
            outerSplit.Dock = DockStyle.Fill;
            outerSplit.Orientation = Orientation.Vertical;
            outerSplit.SplitterWidth = 5;
            outerSplit.BackColor = Theme.Border;
            outerSplit.Panel1MinSize = 140;
            outerSplit.Panel2MinSize = 250;
            outerSplit.Panel1.Controls.Add(sidebar);
            outerSplit.Panel2.Controls.Add(innerSplit);
            try
            {
                if (outerSplit.Width >= 450) outerSplit.SplitterDistance = 240;
            }
            catch { }

            Controls.Add(outerSplit);
            Controls.Add(statusBar);
            Controls.Add(commandStrip);
            FillList();
        }

        protected override void OnSizeChanged(EventArgs e)
        {
            base.OnSizeChanged(e);
            if (innerSplit != null && innerSplit.Width > 500)
            {
                try
                {
                    int target = innerSplit.Width - 280;
                    if (target >= innerSplit.Panel1MinSize && target <= innerSplit.Width - innerSplit.Panel2MinSize)
                        innerSplit.SplitterDistance = target;
                }
                catch { }
            }
        }

        static Control MakeSeparator()
        {
            var sep = new Panel();
            sep.Width = 1;
            sep.Height = 22;
            sep.Margin = new Padding(6, 4, 6, 2);
            sep.BackColor = Theme.Border;
            return sep;
        }

        public void RefreshTable(string id)
        {
            if (current != null && current.Id == id) Bind();
        }

        static NumericUpDown NumberBox(decimal value, decimal min, decimal max, int decimals)
        {
            var box = new NumericUpDown();
            box.Minimum = min;
            box.Maximum = max;
            box.DecimalPlaces = decimals;
            box.Value = value;
            box.Width = 80;
            return box;
        }

        static Label LabelFor(string text)
        {
            var label = new Label();
            label.Text = text;
            label.AutoSize = true;
            label.Padding = new Padding(0, 8, 0, 0);
            return label;
        }

        public string ChecksumText
        {
            get { return checksumMark.Text; }
        }

        public TuneDocument Document
        {
            get { return document; }
        }

        public bool CommitTable(TuneTable table, string message)
        {
            if (!romBacked) return false;
            if (message == null) message = "";
            return WriteTable(table, message);
        }

        public long RomOrigin
        {
            get { return romOrigin; }
        }

        public bool HasRom
        {
            get { return rom != null && rom.Bytes != null && rom.Bytes.Length > 0; }
        }

        public void UpdateTrace(double rpm, double map, double load)
        {
            if (InvokeRequired)
            {
                try { BeginInvoke((Action)delegate { UpdateTrace(rpm, map, load); }); }
                catch { }
                return;
            }
            double yVal = map;
            if (current != null && current.YName != null)
            {
                if (current.YName.IndexOf("load", StringComparison.OrdinalIgnoreCase) >= 0)
                    yVal = load;
                else if (current.YName.IndexOf("map", StringComparison.OrdinalIgnoreCase) >= 0 ||
                         current.YName.IndexOf("kpa", StringComparison.OrdinalIgnoreCase) >= 0)
                    yVal = map;
                else if (map <= 0 && load > 0)
                    yVal = load;
            }
            decimal clampedX = (decimal)Math.Max((double)traceX.Minimum, Math.Min((double)traceX.Maximum, rpm));
            decimal clampedY = (decimal)Math.Max((double)traceY.Minimum, Math.Min((double)traceY.Maximum, yVal));
            if (traceX.Value != clampedX) traceX.Value = clampedX;
            if (traceY.Value != clampedY) traceY.Value = clampedY;
            Trace();
        }

        static Font BadgeFont()
        {
            var face = new Font("Bahnschrift", 11f, FontStyle.Bold, GraphicsUnit.Point);
            if (face.Name == "Bahnschrift") return face;
            face.Dispose();
            return new Font("Segoe UI", 11f, FontStyle.Bold, GraphicsUnit.Point);
        }

        void SetMark(string text)
        {
            checksumMark.Text = text == null || text.Length == 0 ? "Checksum Not Set" : text;
            if (checksumMark.Text == PlatformSecurityAndChecksum.ValidText)
            {
                checksumMark.BackColor = Theme.ValidGreen;
                checksumMark.ForeColor = Theme.Accent;
            }
            else if (checksumMark.Text == PlatformSecurityAndChecksum.UpdatedText)
            {
                checksumMark.BackColor = Theme.WarningAmber;
                checksumMark.ForeColor = Color.FromArgb(90, 62, 18);
            }
            else
            {
                checksumMark.BackColor = Theme.Panel;
                checksumMark.ForeColor = Theme.Muted;
            }
        }

        void RefreshChecksum()
        {
            if (rom == null)
            {
                SetMark("");
                return;
            }
            IChecksum checksum = PlatformSecurityAndChecksum.ChecksumFor(null, rom.Bytes, romOrigin);
            if (checksum != null && checksum.Verify(rom.Bytes)) SetMark(PlatformSecurityAndChecksum.ValidText);
            else SetMark("");
        }

        static Button MakeButton(string text, EventHandler click)
        {
            var button = new Button();
            button.Text = text;
            button.Height = 28;
            button.AutoSize = true;
            button.FlatStyle = FlatStyle.Flat;
            button.FlatAppearance.BorderSize = 0;
            button.BackColor = Theme.Accent;
            button.ForeColor = Theme.TextLight;
            button.Margin = new Padding(2, 2, 2, 2);
            button.AccessibleRole = AccessibleRole.PushButton;
            button.Click += click;
            return button;
        }

        void FillList()
        {
            int rank = level.SelectedIndex;
            string keep = current == null ? "ve" : current.Id;
            tree.BeforeSelect -= RejectGroup;
            tree.AfterSelect -= ShowSelected;
            tree.Nodes.Clear();
            TreeNode match = null;
            foreach (TuneTable table in document.Tables)
            {
                if (!Listed(table, rank)) continue;
                TreeNode group = null;
                foreach (TreeNode node in tree.Nodes)
                    if (node.Text == table.Domain) group = node;
                if (group == null)
                {
                    group = new TreeNode(table.Domain);
                    tree.Nodes.Add(group);
                }
                var child = new TreeNode(table.Name);
                child.Tag = table;
                group.Nodes.Add(child);
                if (table.Id == keep) match = child;
            }
            tree.ExpandAll();
            if (match == null && tree.Nodes.Count > 0 && tree.Nodes[0].Nodes.Count > 0)
                match = tree.Nodes[0].Nodes[0];
            tree.SelectedNode = match;
            tree.BeforeSelect += RejectGroup;
            tree.AfterSelect += ShowSelected;
            if (match == null)
            {
                current = null;
                Bind();
            }
            else ShowSelected(tree, new TreeViewEventArgs(match));
            RebuildSwitches();
        }

        bool Listed(TuneTable table, int rank)
        {
            if (table.LevelRank() > rank) return false;
            if (parameterFilter.Length == 0) return true;
            if (ContainsText(table.Name, parameterFilter)) return true;
            if (ContainsText(table.Domain, parameterFilter)) return true;
            if (ContainsText(table.Unit, parameterFilter)) return true;
            return false;
        }

        static bool ContainsText(string text, string part)
        {
            if (text == null || part == null || part.Length == 0) return false;
            return text.IndexOf(part, StringComparison.OrdinalIgnoreCase) >= 0;
        }

        void RejectGroup(object sender, TreeViewCancelEventArgs e)
        {
            if (e.Node == null || !(e.Node.Tag is TuneTable)) e.Cancel = true;
        }

        void ShowSelected(object sender, TreeViewEventArgs e)
        {
            current = e.Node == null ? null : e.Node.Tag as TuneTable;
            Bind();
        }

        void Bind()
        {
            grid.Columns.Clear();
            grid.Rows.Clear();
            mesh.Show(current);
            MeasureHeat();
            if (current == null) return;
            for (int x = 0; x < current.X.Length; x++)
            {
                var column = new DataGridViewTextBoxColumn();
                column.HeaderText = current.X[x].ToString(CultureInfo.InvariantCulture);
                column.Width = 64;
                grid.Columns.Add(column);
            }
            for (int y = 0; y < current.Values.Length; y++)
            {
                var cells = new object[current.Values[y].Length];
                for (int x = 0; x < cells.Length; x++) cells[x] = CellDelta.FormatNumber(current, current.Values[y][x]);
                int row = grid.Rows.Add(cells);
                string header = current.Kind == "curve" || current.Kind == "scalar" ? current.Unit : current.Y[y].ToString(CultureInfo.InvariantCulture);
                grid.Rows[row].HeaderCell.Value = header;
            }
            status.Text = current.Domain + " / " + current.Name + " (" + current.Unit + "). " + current.XName + " across, " + current.YName + " down. " + current.Level + ". + and - step by the amount. The command box takes +5%, -2%, or =value. I interpolates the selection.";
            Trace();
        }

        void FormatCell(object sender, DataGridViewCellFormattingEventArgs e)
        {
            if (current == null || e.RowIndex < 0 || e.ColumnIndex < 0) return;
            if (e.RowIndex >= current.Values.Length || e.ColumnIndex >= current.Values[e.RowIndex].Length) return;
            double value = current.Values[e.RowIndex][e.ColumnIndex];
            if (grid.IsCurrentCellInEditMode && grid.CurrentCell != null
                && grid.CurrentCell.RowIndex == e.RowIndex && grid.CurrentCell.ColumnIndex == e.ColumnIndex)
            {
                e.Value = CellDelta.FormatNumber(current, value);
                e.FormattingApplied = true;
                return;
            }
            double basis = current.Base[e.RowIndex][e.ColumnIndex];
            double unit = (value - heatMin) / heatSpan;
            e.Value = CellPaint.Apply(e.CellStyle, current, value, basis, unit, compare.Checked, boldFont, grid.Font);
            e.FormattingApplied = true;
        }

        void MeasureHeat()
        {
            heatMin = 0;
            heatSpan = 1;
            if (current == null || current.Values == null || current.Values.Length == 0 || current.Values[0].Length == 0) return;
            heatMin = current.Values[0][0];
            double max = heatMin;
            for (int y = 0; y < current.Values.Length; y++)
            {
                for (int x = 0; x < current.Values[y].Length; x++)
                {
                    double value = current.Values[y][x];
                    if (value < heatMin) heatMin = value;
                    if (value > max) max = value;
                }
            }
            heatSpan = max - heatMin;
            if (heatSpan < 0.0001) heatSpan = 1;
        }

        void Nudge(Keys key)
        {
            if (current == null || grid.IsCurrentCellInEditMode) return;
            if (key == Keys.Up)
                Run(delegate { TableMath.Add(current, grid.SelectedCells, (double)amount.Value); }, "Stepped the selected cells up.");
            else if (key == Keys.Down)
                Run(delegate { TableMath.Add(current, grid.SelectedCells, -(double)amount.Value); }, "Stepped the selected cells down.");
            else if (key == Keys.Right)
                Run(delegate { TableMath.Scale(current, grid.SelectedCells, (double)amount.Value); }, "Scaled the selected cells up.");
            else if (key == Keys.Left)
                Run(delegate { TableMath.Scale(current, grid.SelectedCells, -(double)amount.Value); }, "Scaled the selected cells down.");
        }

        void CellEdited(object sender, DataGridViewCellEventArgs e)
        {
            if (current == null) return;
            object raw = grid.Rows[e.RowIndex].Cells[e.ColumnIndex].Value;
            double value;
            if (raw != null && double.TryParse(raw.ToString(), NumberStyles.Float, CultureInfo.InvariantCulture, out value))
                current.Values[e.RowIndex][e.ColumnIndex] = value;
            Save("Edited one cell.");
        }

        void AddClicked(object sender, EventArgs e)
        {
            Run(delegate { TableMath.Add(current, grid.SelectedCells, (double)amount.Value); }, "Added " + amount.Value.ToString(CultureInfo.InvariantCulture) + " to the selected cells.");
        }

        void ScaleClicked(object sender, EventArgs e)
        {
            Run(delegate { TableMath.Scale(current, grid.SelectedCells, (double)amount.Value); }, "Scaled the selected cells by " + amount.Value.ToString(CultureInfo.InvariantCulture) + " percent.");
        }

        void Run(Action action, string message)
        {
            if (current == null) return;
            action();
            Save(message);
        }

        public void ApplyCellCommand(string text)
        {
            if (current == null)
            {
                status.Text = "Choose a table before editing cells.";
                return;
            }
            try { CellCommand.Run(current, grid.SelectedCells, text, (double)amount.Value); }
            catch (Exception ex)
            {
                status.Text = ex.Message;
                return;
            }
            string shown = text == null ? "" : text.Trim();
            if (shown.Length == 0) shown = "the amount";
            Save("Applied " + shown + ".");
        }

        public void StepSelection(bool up)
        {
            double step = Math.Abs((double)amount.Value);
            if (step < 0.0000001) step = 1;
            if (!up) step = -step;
            Run(delegate { TableMath.Add(current, grid.SelectedCells, step); }, up ? "Stepped the selected cells up." : "Stepped the selected cells down.");
        }

        public void InterpolateSelection()
        {
            Run(delegate { TableMath.Interpolate(current, grid.SelectedCells); }, "Interpolated the selected block from its corners.");
        }

        public byte[] CopyRom()
        {
            if (rom == null || rom.Bytes == null) return new byte[0];
            return (byte[])rom.Bytes.Clone();
        }

        void ApplyClicked(object sender, EventArgs e)
        {
            ApplyCellCommand(commandBox.Text);
        }

        void CommandKeyDown(object sender, KeyEventArgs e)
        {
            if (e.KeyCode != Keys.Enter) return;
            ApplyCellCommand(commandBox.Text);
            e.Handled = true;
            e.SuppressKeyPress = true;
        }

        void GridGesture(string name)
        {
            if (name == "increment") StepSelection(true);
            else if (name == "decrement") StepSelection(false);
            else if (name == "set") BeginSetValue();
            else if (name == "interpolate") InterpolateSelection();
        }

        void BeginSetValue()
        {
            string text = commandBox.Text == null ? "" : commandBox.Text.Trim();
            if (text.Length == 0 || text[0] != '=') commandBox.Text = "=";
            commandBox.Focus();
            commandBox.SelectionStart = commandBox.Text.Length;
        }

        void ShowTableView()
        {
            showSwitches = false;
            grid.Visible = true;
            mesh.Visible = true;
            if (switchList != null && switchList.Parent != null) switchList.Parent.Visible = false;
            PaintViewButtons();
        }

        void ShowSwitchView()
        {
            showSwitches = true;
            grid.Visible = false;
            mesh.Visible = false;
            if (switchList != null && switchList.Parent != null)
            {
                switchList.Parent.Visible = true;
                switchList.Parent.BringToFront();
            }
            RebuildSwitches();
            PaintViewButtons();
        }

        void PaintViewButtons()
        {
            Color on = Theme.Accent;
            Color off = Theme.Panel;
            Color onText = Theme.TextLight;
            Color offText = Theme.Ink;
            tableViewButton.BackColor = showSwitches ? off : on;
            tableViewButton.ForeColor = showSwitches ? offText : onText;
            switchViewButton.BackColor = showSwitches ? on : off;
            switchViewButton.ForeColor = showSwitches ? onText : offText;
        }

        void RebuildSwitches()
        {
            if (switchList == null) return;
            fillingSwitches = true;
            while (switchList.Controls.Count > 0)
            {
                Control child = switchList.Controls[0];
                switchList.Controls.RemoveAt(0);
                child.Dispose();
            }
            int top = 4;
            int width = switchList.ClientSize.Width - 28;
            if (width < 180) width = 220;
            string domain = null;
            bool any = false;
            int rank = level.SelectedIndex;
            foreach (TuneTable table in document.Tables)
            {
                if (table.LevelRank() > rank || !IsSwitch(table)) continue;
                int cells = SwitchCells(table);
                for (int y = 0; y < table.Values.Length; y++)
                {
                    for (int x = 0; x < table.Values[y].Length; x++)
                    {
                        string label = SwitchLabel(table, y, x, cells);
                        if (!SwitchMatches(label)) continue;
                        if (domain != table.Domain)
                        {
                            domain = table.Domain;
                            var header = new Label();
                            header.Text = domain;
                            header.Font = boldFont;
                            header.AutoSize = false;
                            header.Height = 24;
                            header.Width = width;
                            header.Location = new Point(8, top);
                            header.ForeColor = Color.FromArgb(23, 33, 28);
                            switchList.Controls.Add(header);
                            top += 26;
                        }
                        var box = new CheckBox();
                        box.Text = label;
                        box.AccessibleName = label;
                        box.AutoSize = false;
                        box.Height = 24;
                        box.Width = width;
                        box.Location = new Point(12, top);
                        box.Checked = table.Values[y][x] >= 0.5;
                        var point = new SwitchPoint();
                        point.Table = table;
                        point.Y = y;
                        point.X = x;
                        box.Tag = point;
                        box.CheckedChanged += SwitchToggled;
                        switchList.Controls.Add(box);
                        top += 26;
                        any = true;
                    }
                }
            }
            if (!any)
            {
                var empty = new Label();
                empty.Text = romBacked
                    ? "No flags or one-byte switches match."
                    : "Open a ROM and an XDF to edit switches.";
                empty.AutoSize = false;
                empty.Height = 48;
                empty.Width = width;
                empty.Location = new Point(8, top);
                switchList.Controls.Add(empty);
                top += 52;
            }
            switchList.AutoScrollMinSize = new Size(0, top + 8);
            fillingSwitches = false;
        }

        static bool IsSwitch(TuneTable table)
        {
            if (table.Binding == null || table.Binding.Region == null || table.Values == null || table.Values.Length == 0) return false;
            if (table.Binding.Flag) return true;
            XdfRegion region = table.Binding.Region;
            if (region.Float || region.Bits != 8) return false;
            int rows = table.Values.Length;
            int cols = table.Values[0].Length;
            int cells = rows * cols;
            if (cells < 1 || cells > 32) return false;
            if (rows > 1 && cols > 1) return false;
            return true;
        }

        static int SwitchCells(TuneTable table)
        {
            int count = 0;
            for (int y = 0; y < table.Values.Length; y++) count += table.Values[y].Length;
            return count;
        }

        bool SwitchMatches(string label)
        {
            if (switchFilter.Length == 0) return true;
            return ContainsText(label, switchFilter);
        }

        static string SwitchLabel(TuneTable table, int y, int x, int cells)
        {
            string label = table.Domain + " / " + table.Name;
            if (cells == 1) return label;
            string axis;
            if (table.Values.Length == 1 && table.X != null && x < table.X.Length)
                axis = CellDelta.FormatNumber(table, table.X[x]);
            else if (table.Y != null && y < table.Y.Length && table.Values.Length > 1)
                axis = CellDelta.FormatNumber(table, table.Y[y]);
            else axis = x.ToString(CultureInfo.InvariantCulture);
            return label + " / " + axis;
        }

        void SwitchToggled(object sender, EventArgs e)
        {
            if (fillingSwitches) return;
            CheckBox box = sender as CheckBox;
            if (box == null) return;
            SwitchPoint point = box.Tag as SwitchPoint;
            if (point == null || point.Table == null) return;
            point.Table.Values[point.Y][point.X] = box.Checked ? 1 : 0;
            WriteTable(point.Table, "Set " + box.Text + ".");
        }

        void RefreshSwitchChecks()
        {
            if (switchList == null) return;
            fillingSwitches = true;
            foreach (Control control in switchList.Controls)
            {
                CheckBox box = control as CheckBox;
                if (box == null) continue;
                SwitchPoint point = box.Tag as SwitchPoint;
                if (point == null || point.Table == null || point.Table.Values == null) continue;
                if (point.Y < 0 || point.Y >= point.Table.Values.Length) continue;
                if (point.X < 0 || point.X >= point.Table.Values[point.Y].Length) continue;
                box.Checked = point.Table.Values[point.Y][point.X] >= 0.5;
            }
            fillingSwitches = false;
        }

        void OpenRomClicked(object sender, EventArgs e)
        {
            var dialog = new OpenFileDialog();
            dialog.Title = "Open ROM";
            dialog.Filter = "Calibration binaries (*.bin;*.cuw)|*.bin;*.cuw|BIN files (*.bin)|*.bin|Toyota CUW (*.cuw)|*.cuw|All files (*.*)|*.*";
            if (dialog.ShowDialog(FindForm()) != DialogResult.OK) return;
            try { OpenRomFile(dialog.FileName); }
            catch (Exception ex) { status.Text = ex.Message; }
        }

        void OpenXdfClicked(object sender, EventArgs e)
        {
            var dialog = new OpenFileDialog();
            dialog.Title = "Open XDF";
            dialog.Filter = "TunerPro definitions (*.xdf)|*.xdf|All files (*.*)|*.*";
            if (dialog.ShowDialog(FindForm()) != DialogResult.OK) return;
            try { OpenDefinitionFile(dialog.FileName); }
            catch (Exception ex) { status.Text = ex.Message; }
        }

        void SaveRomClicked(object sender, EventArgs e)
        {
            if (rom == null)
            {
                status.Text = "Open a ROM before saving one.";
                return;
            }
            var dialog = new SaveFileDialog();
            dialog.Title = "Save ROM";
            dialog.Filter = "BIN files (*.bin)|*.bin|All files (*.*)|*.*";
            string name = Path.GetFileName(romPath);
            if (romPath.EndsWith(".cuw", StringComparison.OrdinalIgnoreCase))
                name = Path.GetFileNameWithoutExtension(romPath) + ".bin";
            dialog.FileName = name;
            if (dialog.ShowDialog(FindForm()) != DialogResult.OK) return;
            try { SaveRomFile(dialog.FileName); }
            catch (Exception ex) { status.Text = ex.Message; }
        }

        public void SaveRomFile(string path)
        {
            if (rom == null) throw new InvalidOperationException("Open a ROM before saving one.");
            IChecksum checksum = PlatformSecurityAndChecksum.ChecksumFor(null, rom.Bytes, romOrigin);
            if (checksum == null)
            {
                SetMark("");
                throw new InvalidOperationException("No checksum range is configured.");
            }
            byte[] before = (byte[])rom.Bytes.Clone();
            rom.Save(path, checksum);
            romPath = path;
            if (romBacked && document != null)
            {
                foreach (TuneTable table in document.Tables)
                    if (table.Binding != null) table.Binding.ReadInto(rom.Bytes, table);
                Bind();
            }
            SetMark(PlatformSecurityAndChecksum.Mark(before, rom.Bytes));
            status.Text = "Saved " + Path.GetFileName(path) + ". " + checksumMark.Text + ".";
        }

        public void OpenRomFile(string path)
        {
            RomFile loaded = CalibrationFiles.Open(path);
            rom = loaded.Image;
            romOrigin = loaded.Origin;
            romNote = loaded.Note;
            romPath = path;
            if (definition != null) BindLoaded();
            else status.Text = "Loaded ROM " + Path.GetFileName(path) + " (" + rom.Bytes.Length.ToString(CultureInfo.InvariantCulture) + " bytes). Open an XDF to fill the parameter tree.";
            RefreshChecksum();
        }

        public void OpenDefinitionFile(string path)
        {
            XdfDocument parsed = XdfDocument.Load(path);
            definition = parsed;
            if (rom != null) BindLoaded();
            else status.Text = "Loaded definition " + Path.GetFileName(path) + " (" + parsed.Items.Count.ToString(CultureInfo.InvariantCulture) + " parameters). Open a ROM to fill the parameter tree.";
        }

        void BindLoaded()
        {
            XdfBindResult result = definition.Bind(rom.Bytes, romOrigin);
            if (result.Document.Tables.Count == 0)
            {
                status.Text = result.Errors.Count == 0
                    ? "The definition has no parameters in this ROM."
                    : result.Errors[0];
                return;
            }
            document = result.Document;
            romBacked = true;
            current = null;
            string header = romNote.Replace("\r", " ").Replace("\n", " ").Trim();
            if (header.Length > 160) header = header.Substring(0, 160);
            note.Text = "ROM " + Path.GetFileName(romPath) + " (" + rom.Bytes.Length.ToString(CultureInfo.InvariantCulture)
                + " bytes) with " + definition.Title + ". " + result.Document.Tables.Count.ToString(CultureInfo.InvariantCulture)
                + " parameters. Categories are folders. Switches edit flags in the ROM buffer. Edits stay there until you save the ROM. Save ROM updates a configured checksum. Nothing is written to an ECU. " + header;
            FillList();
            if (result.Errors.Count > 0)
                status.Text = result.Errors.Count.ToString(CultureInfo.InvariantCulture) + " parameters were skipped. " + result.Errors[0];
            RefreshChecksum();
        }

        void Save(string message)
        {
            if (romBacked)
            {
                WriteThrough(message);
                return;
            }
            document.Write(savePath);
            Bind();
            status.Text = message;
        }

        void WriteThrough(string message)
        {
            WriteTable(current, message);
        }

        bool WriteTable(TuneTable table, string message)
        {
            if (rom == null || table == null || table.Binding == null)
            {
                status.Text = message;
                RefreshSwitchChecks();
                return false;
            }
            byte[] trial = (byte[])rom.Bytes.Clone();
            try
            {
                table.Binding.WriteTable(trial, table);
                rom.Bytes = trial;
                foreach (TuneTable item in document.Tables)
                    if (item.Binding != null) item.Binding.ReadInto(rom.Bytes, item);
            }
            catch (Exception ex)
            {
                if (table.Binding != null) table.Binding.ReadInto(rom.Bytes, table);
                if (current != null) Bind();
                RefreshSwitchChecks();
                status.Text = ex.Message;
                return false;
            }
            if (current != null) Bind();
            status.Text = message + " The ROM buffer was updated.";
            RefreshChecksum();
            RefreshSwitchChecks();
            return true;
        }

        void Trace()
        {
            if (current == null || grid.Columns.Count == 0 || grid.Rows.Count == 0) return;
            int x = TableMath.Nearest(current.X, (double)traceX.Value);
            int y = current.Y.Length > 1 ? TableMath.Nearest(current.Y, (double)traceY.Value) : 0;
            if (y < grid.Rows.Count && x < grid.Columns.Count)
            {
                grid.CurrentCell = grid.Rows[y].Cells[x];
                mesh.SetActiveCell(y, x);
                if (telemReadout != null)
                {
                    telemReadout.Text = string.Format("RPM: {0:F0}   MAP: {1:F1} kPa\r\nTracked Cell: Row {2}, Col {3}",
                        (double)traceX.Value, (double)traceY.Value, y, x);
                }
            }
        }
    }

    sealed class LogPanel : UserControl
    {
        readonly CalibratePanel calibrate;
        readonly string savePath;
        readonly LogChart chart;
        readonly TextBox formula;
        readonly Label result;
        readonly Button startLive;
        readonly Button stopLive;
        readonly Button exportCsv;
        LogTable log;
        LiveTelemetryPoller livePoller;

        public LogPanel(TuneDocument document, CalibratePanel calibrate, string savePath)
        {
            if (document == null) throw new ArgumentNullException("document");
            this.calibrate = calibrate;
            this.savePath = savePath;
            Dock = DockStyle.Fill;
            BackColor = Color.FromArgb(247, 248, 246);

            var title = new Label();
            title.Dock = DockStyle.Top;
            title.Height = 28;
            title.Font = new Font(Font, FontStyle.Bold);
            title.Text = "Log";

            var tools = new FlowLayoutPanel();
            tools.Dock = DockStyle.Top;
            tools.Height = 44;
            tools.WrapContents = true;

            var open = new Button();
            open.Text = "Open log";
            open.Height = 30;
            open.AutoSize = true;
            open.FlatStyle = FlatStyle.Flat;
            open.FlatAppearance.BorderSize = 0;
            open.BackColor = Color.FromArgb(12, 110, 74);
            open.ForeColor = Color.FromArgb(244, 250, 247);
            open.AccessibleRole = AccessibleRole.PushButton;
            open.Click += OpenClicked;

            var suggest = new Button();
            suggest.Text = "Suggest VE";
            suggest.Height = 30;
            suggest.AutoSize = true;
            suggest.FlatStyle = FlatStyle.Flat;
            suggest.FlatAppearance.BorderSize = 0;
            suggest.BackColor = Color.FromArgb(12, 110, 74);
            suggest.ForeColor = Color.FromArgb(244, 250, 247);
            suggest.AccessibleRole = AccessibleRole.PushButton;
            suggest.Click += SuggestClicked;

            var calculate = new Button();
            calculate.Text = "Calculate channel";
            calculate.Height = 30;
            calculate.AutoSize = true;
            calculate.FlatStyle = FlatStyle.Flat;
            calculate.FlatAppearance.BorderSize = 0;
            calculate.BackColor = Color.FromArgb(12, 110, 74);
            calculate.ForeColor = Color.FromArgb(244, 250, 247);
            calculate.AccessibleRole = AccessibleRole.PushButton;
            calculate.Click += CalculateClicked;

            startLive = new Button();
            startLive.Text = "Start Live Log";
            startLive.Height = 30;
            startLive.AutoSize = true;
            startLive.FlatStyle = FlatStyle.Flat;
            startLive.FlatAppearance.BorderSize = 0;
            startLive.BackColor = Color.FromArgb(12, 110, 74);
            startLive.ForeColor = Color.FromArgb(244, 250, 247);
            startLive.AccessibleRole = AccessibleRole.PushButton;
            startLive.Click += StartLiveClicked;

            stopLive = new Button();
            stopLive.Text = "Stop Live Log";
            stopLive.Height = 30;
            stopLive.AutoSize = true;
            stopLive.FlatStyle = FlatStyle.Flat;
            stopLive.FlatAppearance.BorderSize = 0;
            stopLive.BackColor = Color.FromArgb(223, 227, 222);
            stopLive.ForeColor = Color.FromArgb(23, 33, 28);
            stopLive.AccessibleRole = AccessibleRole.PushButton;
            stopLive.Enabled = false;
            stopLive.Click += StopLiveClicked;

            exportCsv = new Button();
            exportCsv.Text = "Export CSV";
            exportCsv.Height = 30;
            exportCsv.AutoSize = true;
            exportCsv.FlatStyle = FlatStyle.Flat;
            exportCsv.FlatAppearance.BorderSize = 0;
            exportCsv.BackColor = Color.FromArgb(12, 110, 74);
            exportCsv.ForeColor = Color.FromArgb(244, 250, 247);
            exportCsv.AccessibleRole = AccessibleRole.PushButton;
            exportCsv.Click += ExportCsvClicked;

            tools.Controls.Add(open);
            tools.Controls.Add(startLive);
            tools.Controls.Add(stopLive);
            tools.Controls.Add(exportCsv);
            tools.Controls.Add(suggest);
            tools.Controls.Add(calculate);

            formula = new TextBox();
            formula.Dock = DockStyle.Top;
            formula.Text = "lambda / target";
            formula.AccessibleName = "Channel formula";
            chart = new LogChart();
            chart.Dock = DockStyle.Fill;
            result = new Label();
            result.Dock = DockStyle.Bottom;
            result.Height = 56;
            result.Text = "Open a CSV log with rpm, load, lambda, and target columns. Suggestions stay in the sample table.";

            Controls.Add(chart);
            Controls.Add(result);
            Controls.Add(formula);
            Controls.Add(tools);
            Controls.Add(title);
        }

        void StartLiveClicked(object sender, EventArgs e)
        {
            if (!VehicleLink.Current.IsConnected)
            {
                result.Text = "Connect the cable on the Cable tab before starting live logging.";
                return;
            }
            if (log == null)
            {
                log = new LogTable();
                log.Headers = ObdTelemetrySample.Headers();
            }
            if (livePoller == null)
            {
                livePoller = new LiveTelemetryPoller(OnLiveSample);
            }
            livePoller.Start();
            startLive.Enabled = false;
            startLive.BackColor = Color.FromArgb(223, 227, 222);
            startLive.ForeColor = Color.FromArgb(23, 33, 28);
            stopLive.Enabled = true;
            stopLive.BackColor = Color.FromArgb(12, 110, 74);
            stopLive.ForeColor = Color.FromArgb(244, 250, 247);
            result.Text = "Live logging started. Polling SAE J1979 Mode 01 telemetry.";
        }

        void StopLiveClicked(object sender, EventArgs e)
        {
            if (livePoller != null)
            {
                livePoller.Stop();
            }
            startLive.Enabled = true;
            startLive.BackColor = Color.FromArgb(12, 110, 74);
            startLive.ForeColor = Color.FromArgb(244, 250, 247);
            stopLive.Enabled = false;
            stopLive.BackColor = Color.FromArgb(223, 227, 222);
            stopLive.ForeColor = Color.FromArgb(23, 33, 28);
            result.Text = "Live logging stopped. " + (log == null ? 0 : log.Rows.Count) + " rows recorded.";
        }

        void OnLiveSample(ObdTelemetrySample sample)
        {
            if (sample == null) return;
            if (InvokeRequired)
            {
                try { BeginInvoke((Action)delegate { OnLiveSample(sample); }); }
                catch { }
                return;
            }
            if (log == null)
            {
                log = new LogTable();
                log.Headers = ObdTelemetrySample.Headers();
            }
            log.Rows.Add(sample.ToRow());
            if (log.Rows.Count > 10000)
            {
                log.Rows.RemoveRange(0, 1000);
            }
            chart.ShowLog(log);
            calibrate.UpdateTrace(sample.Rpm, sample.MapKpa, sample.LoadPct);
            result.Text = "Live: " + log.Rows.Count + " rows. RPM: " + sample.Rpm.ToString("0", CultureInfo.InvariantCulture)
                + "  MAP: " + sample.MapKpa.ToString("0", CultureInfo.InvariantCulture) + " kPa"
                + "  Load: " + sample.LoadPct.ToString("0.0", CultureInfo.InvariantCulture) + "%"
                + "  Advance: " + sample.TimingAdvance.ToString("0.0", CultureInfo.InvariantCulture) + "°"
                + "  ECT: " + sample.CoolantC.ToString("0", CultureInfo.InvariantCulture) + "°C"
                + "  Lambda: " + sample.Lambda.ToString("0.00", CultureInfo.InvariantCulture);
        }

        void ExportCsvClicked(object sender, EventArgs e)
        {
            if (log == null || log.Rows.Count == 0)
            {
                result.Text = "No log data to export.";
                return;
            }
            using (var dialog = new SaveFileDialog())
            {
                dialog.Title = "Export CSV Log";
                dialog.Filter = "CSV logs (*.csv)|*.csv|All files (*.*)|*.*";
                dialog.FileName = "telemetry-" + DateTime.Now.ToString("yyyyMMdd-HHmmss") + ".csv";
                if (dialog.ShowDialog(this) != DialogResult.OK) return;
                var sb = new StringBuilder();
                sb.AppendLine(string.Join(",", log.Headers));
                foreach (double[] row in log.Rows)
                {
                    var line = new string[row.Length];
                    for (int i = 0; i < row.Length; i++)
                        line[i] = row[i].ToString("0.###", CultureInfo.InvariantCulture);
                    sb.AppendLine(string.Join(",", line));
                }
                File.WriteAllText(dialog.FileName, sb.ToString());
                result.Text = "Exported " + log.Rows.Count + " rows to " + Path.GetFileName(dialog.FileName) + ".";
            }
        }

        void OpenClicked(object sender, EventArgs e)
        {
            var dialog = new OpenFileDialog();
            dialog.Filter = "CSV logs (*.csv)|*.csv|All files (*.*)|*.*";
            if (dialog.ShowDialog(this) != DialogResult.OK) return;
            log = LogTable.Parse(File.ReadAllText(dialog.FileName));
            chart.ShowLog(log);
            result.Text = log.Rows.Count + " rows from " + Path.GetFileName(dialog.FileName) + ".";
        }

        public void LoadText(string text, string name)
        {
            log = LogTable.Parse(text);
            chart.ShowLog(log);
            result.Text = log.Rows.Count + " rows from " + name + ".";
        }

        void CalculateClicked(object sender, EventArgs e)
        {
            if (log == null || log.Rows.Count == 0)
            {
                result.Text = "Open a log first.";
                return;
            }
            try
            {
                double sum = 0;
                double min = 0;
                double max = 0;
                for (int i = 0; i < log.Rows.Count; i++)
                {
                    var variables = new Dictionary<string, double>();
                    for (int c = 0; c < log.Headers.Length; c++) variables[log.Headers[c].ToLowerInvariant()] = log.Rows[i][c];
                    double value = ChannelMath.Eval(formula.Text, variables);
                    if (i == 0) { min = value; max = value; }
                    if (value < min) min = value;
                    if (value > max) max = value;
                    sum += value;
                }
                result.Text = "Channel " + formula.Text + " over " + log.Rows.Count + " rows. Min " + min.ToString("0.###", CultureInfo.InvariantCulture)
                    + ", average " + (sum / log.Rows.Count).ToString("0.###", CultureInfo.InvariantCulture)
                    + ", max " + max.ToString("0.###", CultureInfo.InvariantCulture) + ".";
            }
            catch (Exception ex)
            {
                result.Text = ex.Message;
            }
        }

        void SuggestClicked(object sender, EventArgs e)
        {
            TuneDocument calibration = calibrate.Document;
            TuneTable ve = calibration == null ? null : calibration.Find("ve");
            if (log == null || ve == null)
            {
                result.Text = "Open a log before asking for a VE suggestion.";
                return;
            }
            int rpm = log.Index("rpm");
            int load = log.Index("load");
            int lambda = log.Index("lambda");
            int target = log.Index("target");
            if (rpm < 0 || load < 0 || lambda < 0 || target < 0)
            {
                result.Text = "The log needs rpm, load, lambda, and target columns.";
                return;
            }
            var sum = new double[ve.Values.Length][];
            var count = new int[ve.Values.Length][];
            for (int y = 0; y < ve.Values.Length; y++)
            {
                sum[y] = new double[ve.Values[y].Length];
                count[y] = new int[ve.Values[y].Length];
            }
            int used = 0;
            foreach (double[] row in log.Rows)
            {
                if (row[lambda] <= 0 || row[target] <= 0) continue;
                int x = TableMath.Nearest(ve.X, row[rpm]);
                int y = TableMath.Nearest(ve.Y, row[load]);
                sum[y][x] += row[target] / row[lambda];
                count[y][x]++;
                used++;
            }
            int changed = 0;
            for (int y = 0; y < ve.Values.Length; y++)
            {
                for (int x = 0; x < ve.Values[y].Length; x++)
                {
                    if (count[y][x] == 0) continue;
                    ve.Values[y][x] *= sum[y][x] / count[y][x];
                    changed++;
                }
            }
            if (!calibration.Persist)
            {
                if (!calibrate.CommitTable(ve, "Applied a VE suggestion from the log."))
                {
                    result.Text = "The VE suggestion could not be stored in the ROM buffer.";
                    return;
                }
                result.Text = "Applied a VE suggestion from " + used + " log rows to " + changed + " ROM cells. The ECU was not written.";
                return;
            }
            calibration.Write(savePath);
            calibrate.RefreshTable("ve");
            result.Text = "Applied a VE suggestion from " + used + " log rows to " + changed + " sample cells. The ECU was not written.";
        }
    }

    sealed class LogChart : Control
    {
        LogTable log;
        int hoverRow = -1;
        int mouseX = -1;
        int mouseY = -1;
        bool isHovering = false;

        public LogChart()
        {
            SetStyle(ControlStyles.AllPaintingInWmPaint | ControlStyles.UserPaint | ControlStyles.OptimizedDoubleBuffer | ControlStyles.ResizeRedraw, true);
            DoubleBuffered = true;
            BackColor = Color.FromArgb(24, 28, 26);
        }

        public void ShowLog(LogTable value)
        {
            log = value;
            Invalidate();
        }

        protected override void OnMouseMove(MouseEventArgs e)
        {
            base.OnMouseMove(e);
            mouseX = e.X;
            mouseY = e.Y;
            isHovering = true;
            if (log != null && log.Rows.Count > 1)
            {
                float chartLeft = 55f;
                float chartWidth = Width - chartLeft - 20f;
                if (chartWidth > 0 && e.X >= chartLeft && e.X <= chartLeft + chartWidth)
                {
                    float ratio = (e.X - chartLeft) / chartWidth;
                    hoverRow = (int)Math.Round(ratio * (log.Rows.Count - 1));
                    if (hoverRow < 0) hoverRow = 0;
                    if (hoverRow >= log.Rows.Count) hoverRow = log.Rows.Count - 1;
                }
                else
                {
                    hoverRow = -1;
                }
            }
            Invalidate();
        }

        protected override void OnMouseLeave(EventArgs e)
        {
            base.OnMouseLeave(e);
            isHovering = false;
            hoverRow = -1;
            Invalidate();
        }

        protected override void OnPaint(PaintEventArgs e)
        {
            base.OnPaint(e);
            Graphics g = e.Graphics;
            g.Clear(BackColor);
            g.SmoothingMode = SmoothingMode.AntiAlias;
            g.TextRenderingHint = TextRenderingHint.ClearTypeGridFit;

            using (var borderPen = new Pen(Color.FromArgb(45, 55, 48), 1f))
            {
                g.DrawRectangle(borderPen, 0, 0, Width - 1, Height - 1);
            }

            if (log == null || log.Rows.Count < 2)
            {
                using (var brush = new SolidBrush(Color.FromArgb(120, 140, 130)))
                {
                    g.DrawString("No log loaded. Open a CSV or click 'Start Live Log'.", Font, brush, 16, 16);
                }
                return;
            }

            float chartLeft = 55f;
            float chartRight = Width - 20f;
            float chartTop = 32f;
            float chartBottom = Height - 28f;
            float chartWidth = chartRight - chartLeft;
            float chartHeight = chartBottom - chartTop;

            if (chartWidth <= 10 || chartHeight <= 10) return;

            using (var gridPen = new Pen(Color.FromArgb(40, 50, 44), 1f))
            using (var gridTextBrush = new SolidBrush(Color.FromArgb(120, 140, 130)))
            using (var smFont = new Font(Font.FontFamily, 8f, FontStyle.Regular))
            {
                gridPen.DashStyle = DashStyle.Dash;
                for (int i = 0; i <= 4; i++)
                {
                    float gy = chartTop + chartHeight * (i / 4f);
                    g.DrawLine(gridPen, chartLeft, gy, chartRight, gy);
                    int pct = 100 - i * 25;
                    g.DrawString(pct.ToString(CultureInfo.InvariantCulture) + "%", smFont, gridTextBrush, 12, gy - 6);
                }
            }

            int rpmCol = FindColumn("rpm");
            int mapCol = FindColumn("map");
            int loadCol = FindColumn("load");
            int lambdaCol = FindColumn("lambda");
            int targetCol = FindColumn("target");

            Color rpmColor = Color.FromArgb(235, 80, 80);
            Color mapColor = Color.FromArgb(80, 175, 240);
            Color loadColor = Color.FromArgb(240, 190, 60);
            Color lambdaColor = Color.FromArgb(60, 220, 140);
            Color targetColor = Color.FromArgb(210, 130, 240);

            if (rpmCol >= 0) DrawSeries(g, rpmCol, rpmColor, chartLeft, chartTop, chartWidth, chartHeight);
            if (mapCol >= 0) DrawSeries(g, mapCol, mapColor, chartLeft, chartTop, chartWidth, chartHeight);
            else if (loadCol >= 0) DrawSeries(g, loadCol, loadColor, chartLeft, chartTop, chartWidth, chartHeight);
            if (lambdaCol >= 0) DrawSeries(g, lambdaCol, lambdaColor, chartLeft, chartTop, chartWidth, chartHeight);
            if (targetCol >= 0) DrawSeries(g, targetCol, targetColor, chartLeft, chartTop, chartWidth, chartHeight);

            float legendX = chartLeft + 4;
            using (var smFont = new Font(Font.FontFamily, 8.5f, FontStyle.Bold))
            {
                if (rpmCol >= 0) legendX = DrawLegendBadge(g, "RPM", rpmColor, legendX, 8, smFont);
                if (mapCol >= 0) legendX = DrawLegendBadge(g, "MAP", mapColor, legendX, 8, smFont);
                else if (loadCol >= 0) legendX = DrawLegendBadge(g, "Load", loadColor, legendX, 8, smFont);
                if (lambdaCol >= 0) legendX = DrawLegendBadge(g, "Lambda", lambdaColor, legendX, 8, smFont);
                if (targetCol >= 0) legendX = DrawLegendBadge(g, "Target", targetColor, legendX, 8, smFont);
            }

            if (isHovering && hoverRow >= 0 && hoverRow < log.Rows.Count)
            {
                float cursorX = chartLeft + chartWidth * (hoverRow / (float)(log.Rows.Count - 1));
                using (var crossPen = new Pen(Color.FromArgb(160, 255, 255, 255), 1f))
                {
                    crossPen.DashStyle = DashStyle.Dot;
                    g.DrawLine(crossPen, cursorX, chartTop, cursorX, chartBottom);
                    if (mouseY >= chartTop && mouseY <= chartBottom)
                    {
                        g.DrawLine(crossPen, chartLeft, mouseY, chartRight, mouseY);
                    }
                }

                double[] row = log.Rows[hoverRow];
                var sb = new StringBuilder();
                sb.Append("Row ").Append((hoverRow + 1).ToString(CultureInfo.InvariantCulture)).Append("/").Append(log.Rows.Count.ToString(CultureInfo.InvariantCulture));
                if (rpmCol >= 0 && rpmCol < row.Length) sb.Append(" | RPM: ").Append(row[rpmCol].ToString("0", CultureInfo.InvariantCulture));
                if (mapCol >= 0 && mapCol < row.Length) sb.Append(" | MAP: ").Append(row[mapCol].ToString("0.0", CultureInfo.InvariantCulture)).Append(" kPa");
                if (loadCol >= 0 && loadCol < row.Length) sb.Append(" | Load: ").Append(row[loadCol].ToString("0.0", CultureInfo.InvariantCulture)).Append("%");
                if (lambdaCol >= 0 && lambdaCol < row.Length) sb.Append(" | \u03BB: ").Append(row[lambdaCol].ToString("0.00", CultureInfo.InvariantCulture));
                if (targetCol >= 0 && targetCol < row.Length) sb.Append(" | Target: ").Append(row[targetCol].ToString("0.00", CultureInfo.InvariantCulture));

                string readout = sb.ToString();
                using (var readoutFont = new Font(Font.FontFamily, 8.5f, FontStyle.Regular))
                {
                    SizeF sz = g.MeasureString(readout, readoutFont);
                    float boxW = sz.Width + 14;
                    float boxH = sz.Height + 8;
                    float boxX = cursorX + 8;
                    if (boxX + boxW > chartRight) boxX = cursorX - boxW - 8;
                    float boxY = chartTop + 6;

                    using (var boxBrush = new SolidBrush(Color.FromArgb(220, 16, 20, 18)))
                    using (var borderP = new Pen(Color.FromArgb(80, 100, 90), 1f))
                    using (var textB = new SolidBrush(Color.FromArgb(240, 250, 245)))
                    {
                        g.FillRectangle(boxBrush, boxX, boxY, boxW, boxH);
                        g.DrawRectangle(borderP, boxX, boxY, boxW, boxH);
                        g.DrawString(readout, readoutFont, textB, boxX + 7, boxY + 4);
                    }
                }
            }
        }

        int FindColumn(string term)
        {
            if (log == null || log.Headers == null) return -1;
            for (int i = 0; i < log.Headers.Length; i++)
            {
                if (log.Headers[i].IndexOf(term, StringComparison.OrdinalIgnoreCase) >= 0)
                    return i;
            }
            return -1;
        }

        float DrawLegendBadge(Graphics g, string label, Color color, float x, float y, Font font)
        {
            using (var dotBrush = new SolidBrush(color))
            {
                g.FillEllipse(dotBrush, x, y + 4, 8, 8);
            }
            using (var textBrush = new SolidBrush(Color.FromArgb(210, 220, 215)))
            {
                g.DrawString(label, font, textBrush, x + 12, y);
            }
            SizeF sz = g.MeasureString(label, font);
            return x + 12 + sz.Width + 16;
        }

        void DrawSeries(Graphics graphics, int column, Color color, float left, float top, float width, float height)
        {
            if (column < 0 || column >= log.Headers.Length) return;
            double min = log.Rows[0][column];
            double max = min;
            foreach (double[] row in log.Rows)
            {
                if (column < row.Length)
                {
                    if (row[column] < min) min = row[column];
                    if (row[column] > max) max = row[column];
                }
            }
            if (Math.Abs(max - min) < 0.0001) max = min + 1;

            var points = new PointF[log.Rows.Count];
            for (int i = 0; i < log.Rows.Count; i++)
            {
                double val = column < log.Rows[i].Length ? log.Rows[i][column] : min;
                float x = left + width * (i / (float)(log.Rows.Count - 1));
                float y = top + height - (float)((val - min) / (max - min) * height);
                points[i] = new PointF(x, y);
            }
            using (var pen = new Pen(color, 2f))
            {
                graphics.DrawLines(pen, points);
            }
        }
    }

    sealed class TuneGrid : DataGridView
    {
        public event Action<Keys> Nudge;
        public event Action<string> Gesture;

        protected override bool ProcessCmdKey(ref Message msg, Keys keyData)
        {
            if (!IsCurrentCellInEditMode && Gesture != null)
            {
                Keys code = keyData & Keys.KeyCode;
                bool shift = (keyData & Keys.Shift) == Keys.Shift;
                bool plain = (keyData & (Keys.Control | Keys.Alt)) == 0;
                if (code == Keys.Add || (code == Keys.Oemplus && shift))
                {
                    Gesture("increment");
                    return true;
                }
                if (code == Keys.Subtract || (code == Keys.OemMinus && !shift))
                {
                    Gesture("decrement");
                    return true;
                }
                if (plain && code == Keys.Oemplus && !shift)
                {
                    Gesture("set");
                    return true;
                }
                if (plain && code == Keys.I)
                {
                    Gesture("interpolate");
                    return true;
                }
            }
            return base.ProcessCmdKey(ref msg, keyData);
        }

        protected override bool ProcessDataGridViewKey(KeyEventArgs e)
        {
            if (e.Shift && (e.KeyCode == Keys.Up || e.KeyCode == Keys.Down || e.KeyCode == Keys.Left || e.KeyCode == Keys.Right))
            {
                if (Nudge != null) Nudge(e.KeyCode);
                return true;
            }
            return base.ProcessDataGridViewKey(e);
        }
    }

    sealed class SurfaceMesh : Control
    {
        TuneTable table;
        int activeX = -1;
        int activeY = -1;

        public SurfaceMesh()
        {
            SetStyle(ControlStyles.AllPaintingInWmPaint | ControlStyles.UserPaint | ControlStyles.OptimizedDoubleBuffer | ControlStyles.ResizeRedraw, true);
            BackColor = Color.FromArgb(28, 34, 30);
            DoubleBuffered = true;
        }

        public void Show(TuneTable value)
        {
            table = value;
            Invalidate();
        }

        public void SetActiveCell(int y, int x)
        {
            activeY = y;
            activeX = x;
            Invalidate();
        }

        protected override void OnPaint(PaintEventArgs e)
        {
            base.OnPaint(e);
            Graphics g = e.Graphics;
            g.Clear(BackColor);
            g.SmoothingMode = SmoothingMode.AntiAlias;
            g.TextRenderingHint = TextRenderingHint.ClearTypeGridFit;

            using (var borderPen = new Pen(Color.FromArgb(45, 55, 48), 1f))
            {
                g.DrawRectangle(borderPen, 0, 0, Width - 1, Height - 1);
            }

            if (table == null || table.Values == null || table.Values.Length == 0 || table.Values[0].Length == 0)
            {
                using (var brush = new SolidBrush(Color.FromArgb(120, 140, 130)))
                {
                    g.DrawString("No table selected", Font, brush, 12, 12);
                }
                return;
            }

            int rows = table.Values.Length;
            int cols = table.Values[0].Length;

            double min = table.Values[0][0];
            double max = min;
            for (int r = 0; r < rows; r++)
            {
                for (int c = 0; c < table.Values[r].Length; c++)
                {
                    double v = table.Values[r][c];
                    if (v < min) min = v;
                    if (v > max) max = v;
                }
            }
            double span = max - min;
            if (span < 0.0001) span = 1;

            using (var titleBrush = new SolidBrush(Color.FromArgb(235, 242, 238)))
            using (var boldFont = new Font(Font, FontStyle.Bold))
            {
                g.DrawString(table.Name, boldFont, titleBrush, 10, 8);
            }
            using (var statBrush = new SolidBrush(Color.FromArgb(160, 180, 170)))
            using (var smFont = new Font(Font.FontFamily, 8.5f, FontStyle.Regular))
            {
                string maxStr = "Max: " + CellDelta.FormatNumber(table, max) + (table.Unit.Length > 0 ? " " + table.Unit : "");
                string minStr = "Min: " + CellDelta.FormatNumber(table, min) + (table.Unit.Length > 0 ? " " + table.Unit : "");
                g.DrawString(maxStr, smFont, statBrush, 10, 26);
                g.DrawString(minStr, smFont, statBrush, 10, 40);
            }

            float margin = 20f;
            float topY = 56f;
            float viewW = Width - margin * 2;
            float viewH = Height - topY - margin;
            if (viewW < 40 || viewH < 40) return;

            float cx = Width * 0.5f;
            float cy = topY + viewH * 0.65f;

            float stepX = viewW / (float)(cols + rows + 1);
            float stepY = (viewH * 0.45f) / (float)(cols + rows + 1);
            float maxZHeight = viewH * 0.45f;

            var pts = new PointF[rows, cols];
            var zUnits = new double[rows, cols];

            for (int r = 0; r < rows; r++)
            {
                for (int c = 0; c < cols; c++)
                {
                    double val = c < table.Values[r].Length ? table.Values[r][c] : min;
                    double unit = (val - min) / span;
                    if (unit < 0) unit = 0;
                    if (unit > 1) unit = 1;
                    zUnits[r, c] = unit;

                    float isoX = (c - (cols - 1) * 0.5f) - (r - (rows - 1) * 0.5f);
                    float isoY = ((c - (cols - 1) * 0.5f) + (r - (rows - 1) * 0.5f)) * 0.5f;

                    float px = cx + isoX * stepX * 1.5f;
                    float py = cy + isoY * stepY * 2.0f - (float)unit * maxZHeight;
                    pts[r, c] = new PointF(px, py);
                }
            }

            using (var axisPen = new Pen(Color.FromArgb(50, 70, 60), 1f))
            {
                axisPen.DashStyle = DashStyle.Dash;
                PointF floor00 = new PointF(cx + (-(cols - 1) * 0.5f - -(rows - 1) * 0.5f) * stepX * 1.5f,
                                            cy + (-(cols - 1) * 0.5f + -(rows - 1) * 0.5f) * 0.5f * stepY * 2.0f);
                PointF floorC0 = new PointF(cx + ((cols - 1) * 0.5f - -(rows - 1) * 0.5f) * stepX * 1.5f,
                                            cy + ((cols - 1) * 0.5f + -(rows - 1) * 0.5f) * 0.5f * stepY * 2.0f);
                PointF floorCR = new PointF(cx + ((cols - 1) * 0.5f - (rows - 1) * 0.5f) * stepX * 1.5f,
                                            cy + ((cols - 1) * 0.5f + (rows - 1) * 0.5f) * 0.5f * stepY * 2.0f);
                PointF floor0R = new PointF(cx + (-(cols - 1) * 0.5f - (rows - 1) * 0.5f) * stepX * 1.5f,
                                            cy + (-(cols - 1) * 0.5f + (rows - 1) * 0.5f) * 0.5f * stepY * 2.0f);
                g.DrawLine(axisPen, floor00, floorC0);
                g.DrawLine(axisPen, floorC0, floorCR);
                g.DrawLine(axisPen, floorCR, floor0R);
                g.DrawLine(axisPen, floor0R, floor00);
            }

            using (var wirePen = new Pen(Color.FromArgb(120, 255, 255, 255), 1f))
            {
                for (int r = 0; r < rows - 1; r++)
                {
                    for (int c = 0; c < cols - 1; c++)
                    {
                        PointF[] quad = new PointF[]
                        {
                            pts[r, c],
                            pts[r, c + 1],
                            pts[r + 1, c + 1],
                            pts[r + 1, c]
                        };

                        double avgUnit = (zUnits[r, c] + zUnits[r, c + 1] + zUnits[r + 1, c + 1] + zUnits[r + 1, c]) / 4.0;
                        Color quadColor = HeatColor.Deep(avgUnit);
                        using (var brush = new SolidBrush(Color.FromArgb(215, quadColor.R, quadColor.G, quadColor.B)))
                        {
                            g.FillPolygon(brush, quad);
                        }
                        g.DrawPolygon(wirePen, quad);
                    }
                }
            }

            if (activeY >= 0 && activeY < rows && activeX >= 0 && activeX < cols)
            {
                PointF actP = pts[activeY, activeX];
                float isoX = (activeX - (cols - 1) * 0.5f) - (activeY - (rows - 1) * 0.5f);
                float isoY = ((activeX - (cols - 1) * 0.5f) + (activeY - (rows - 1) * 0.5f)) * 0.5f;
                PointF floorP = new PointF(cx + isoX * stepX * 1.5f, cy + isoY * stepY * 2.0f);

                using (var stemPen = new Pen(Color.FromArgb(255, 240, 100), 1.5f))
                {
                    stemPen.DashStyle = DashStyle.Dot;
                    g.DrawLine(stemPen, floorP, actP);
                }

                using (var outerPen = new Pen(Color.FromArgb(255, 215, 0), 2.5f))
                {
                    g.DrawEllipse(outerPen, actP.X - 5, actP.Y - 5, 10, 10);
                }
                using (var innerBrush = new SolidBrush(Color.White))
                {
                    g.FillEllipse(innerBrush, actP.X - 3, actP.Y - 3, 6, 6);
                }
            }
        }
    }
}
