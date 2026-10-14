using System;
using System.Collections;
using System.Collections.Generic;
using System.Drawing;
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
            created.Write(path);
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
            style.ForeColor = Color.FromArgb(23, 33, 28);
            if (compare)
            {
                double delta = value - basis;
                if (delta > 0.0001) style.BackColor = Color.FromArgb(214, 236, 214);
                else if (delta < -0.0001) style.BackColor = Color.FromArgb(244, 214, 214);
                else style.BackColor = Color.FromArgb(247, 248, 246);
            }
            else style.BackColor = HeatColor.Blend(unit);
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
            BackColor = Color.FromArgb(247, 248, 246);

            note = new Label();
            note.Dock = DockStyle.Top;
            note.Height = 80;
            note.Text = document.Note + " Tables are edited here. Nothing is written to an ECU.";

            var tools = new FlowLayoutPanel();
            tools.Dock = DockStyle.Top;
            tools.Height = 188;
            tools.WrapContents = true;
            tools.Controls.Add(MakeButton("Open ROM", OpenRomClicked));
            tools.Controls.Add(MakeButton("Open XDF", OpenXdfClicked));
            tools.Controls.Add(MakeButton("Save ROM", SaveRomClicked));
            checksumMark = new Label();
            checksumMark.AutoSize = true;
            checksumMark.TextAlign = ContentAlignment.MiddleCenter;
            checksumMark.Padding = new Padding(10, 6, 10, 6);
            checksumMark.Margin = new Padding(8, 3, 4, 3);
            checksumMark.AccessibleName = "Checksum indicator";
            checksumMark.AccessibleRole = AccessibleRole.StaticText;
            Font badgeFont = BadgeFont();
            checksumMark.Font = badgeFont;
            tools.Controls.Add(checksumMark);
            SetMark("");
            level = new ComboBox();
            level.DropDownStyle = ComboBoxStyle.DropDownList;
            level.Width = 130;
            level.Items.AddRange(new object[] { "Beginner", "Intermediate", "Advanced" });
            level.SelectedIndex = 1;
            level.AccessibleName = "Safety level";
            level.SelectedIndexChanged += delegate { FillList(); };
            amount = NumberBox(10, -1000, 1000, 0);
            amount.AccessibleName = "Amount";
            traceX = NumberBox(3000, 0, 20000, 0);
            traceX.AccessibleName = "Trace RPM";
            traceY = NumberBox(60, 0, 500, 0);
            traceY.AccessibleName = "Trace load";
            compare = new CheckBox();
            compare.Text = "Compare";
            compare.AutoSize = true;
            compare.CheckedChanged += delegate { grid.Invalidate(); };
            tools.Controls.Add(LabelFor("Level"));
            tools.Controls.Add(level);
            tools.Controls.Add(MakeButton("Add", AddClicked));
            tools.Controls.Add(MakeButton("Scale %", ScaleClicked));
            tools.Controls.Add(amount);
            tools.Controls.Add(MakeButton("Smooth", delegate { Run(delegate { TableMath.Smooth(current, grid.SelectedCells); }, "Smoothed the selected cells."); }));
            tools.Controls.Add(MakeButton("Interpolate", delegate { InterpolateSelection(); }));
            commandBox = new TextBox();
            commandBox.Width = 110;
            commandBox.AccessibleName = "Cell command";
            commandBox.KeyDown += CommandKeyDown;
            tools.Controls.Add(commandBox);
            tools.Controls.Add(MakeButton("Apply", ApplyClicked));
            tableViewButton = MakeButton("Table", delegate { ShowTableView(); });
            switchViewButton = MakeButton("Switches", delegate { ShowSwitchView(); });
            tools.Controls.Add(tableViewButton);
            tools.Controls.Add(switchViewButton);
            PaintViewButtons();
            tools.Controls.Add(compare);
            tools.Controls.Add(LabelFor("Trace"));
            tools.Controls.Add(traceX);
            tools.Controls.Add(traceY);

            var sidebar = new Panel();
            sidebar.Dock = DockStyle.Left;
            sidebar.Width = 260;
            sidebar.BackColor = BackColor;
            var findLabel = new Label();
            findLabel.Text = "Find";
            findLabel.Dock = DockStyle.Top;
            findLabel.Height = 22;
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
            tree.BackColor = BackColor;
            tree.AccessibleName = "Parameter tree";
            sidebar.Controls.Add(tree);
            sidebar.Controls.Add(searchBox);
            sidebar.Controls.Add(findLabel);

            mesh = new SurfaceMesh();
            mesh.Dock = DockStyle.Right;
            mesh.Width = 260;

            grid = new TuneGrid();
            grid.Dock = DockStyle.Fill;
            grid.AllowUserToAddRows = false;
            grid.AllowUserToDeleteRows = false;
            grid.RowHeadersWidth = 72;
            grid.BackgroundColor = BackColor;
            grid.BorderStyle = BorderStyle.None;
            grid.SelectionMode = DataGridViewSelectionMode.CellSelect;
            grid.CellEndEdit += CellEdited;
            grid.CellFormatting += FormatCell;
            grid.Nudge += Nudge;
            grid.Gesture += GridGesture;
            boldFont = new Font(grid.Font, FontStyle.Bold);
            traceX.ValueChanged += delegate { Trace(); };
            traceY.ValueChanged += delegate { Trace(); };

            status = new Label();
            status.Dock = DockStyle.Bottom;
            status.Height = 36;
            status.Text = "Choose a table.";

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
            switchList.BackColor = BackColor;
            var switchBoard = new Panel();
            switchBoard.Dock = DockStyle.Fill;
            switchBoard.Visible = false;
            switchBoard.BackColor = BackColor;
            switchBoard.AccessibleName = "Switch board";
            switchBoard.Controls.Add(switchList);
            switchBoard.Controls.Add(switchSearch);

            var workspace = new Panel();
            workspace.Dock = DockStyle.Fill;
            workspace.BackColor = BackColor;
            workspace.Controls.Add(grid);
            workspace.Controls.Add(switchBoard);
            workspace.Controls.Add(mesh);

            Controls.Add(workspace);
            Controls.Add(sidebar);
            Controls.Add(status);
            Controls.Add(tools);
            Controls.Add(note);
            FillList();
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

        static Font BadgeFont()
        {
            var face = new Font("Bahnschrift", 11f, FontStyle.Bold, GraphicsUnit.Point);
            if (face.Name == "Bahnschrift") return face;
            face.Dispose();
            return new Font("Segoe UI", 11f, FontStyle.Bold, GraphicsUnit.Point);
        }

        void SetMark(string text)
        {
            checksumMark.Text = text == null ? "" : text;
            if (checksumMark.Text == PlatformSecurityAndChecksum.ValidText)
            {
                checksumMark.BackColor = Color.FromArgb(214, 236, 214);
                checksumMark.ForeColor = Color.FromArgb(12, 110, 74);
            }
            else if (checksumMark.Text == PlatformSecurityAndChecksum.UpdatedText)
            {
                checksumMark.BackColor = Color.FromArgb(244, 228, 196);
                checksumMark.ForeColor = Color.FromArgb(90, 62, 18);
            }
            else
            {
                checksumMark.BackColor = Color.FromArgb(247, 248, 246);
                checksumMark.ForeColor = Color.FromArgb(40, 44, 42);
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
            button.Height = 30;
            button.AutoSize = true;
            button.FlatStyle = FlatStyle.Flat;
            button.FlatAppearance.BorderSize = 0;
            button.BackColor = Color.FromArgb(12, 110, 74);
            button.ForeColor = Color.FromArgb(244, 250, 247);
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
            Color on = Color.FromArgb(12, 110, 74);
            Color off = Color.FromArgb(223, 227, 222);
            Color onText = Color.FromArgb(244, 250, 247);
            Color offText = Color.FromArgb(23, 33, 28);
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
            if (y < grid.Rows.Count && x < grid.Columns.Count) grid.CurrentCell = grid.Rows[y].Cells[x];
        }
    }

    sealed class LogPanel : UserControl
    {
        readonly CalibratePanel calibrate;
        readonly string savePath;
        readonly LogChart chart;
        readonly TextBox formula;
        readonly Label result;
        LogTable log;

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
            tools.Height = 40;
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
            tools.Controls.Add(open);
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

        public LogChart()
        {
            DoubleBuffered = true;
            BackColor = Color.FromArgb(247, 248, 246);
        }

        public void ShowLog(LogTable value)
        {
            log = value;
            Invalidate();
        }

        protected override void OnPaint(PaintEventArgs e)
        {
            base.OnPaint(e);
            if (log == null || log.Rows.Count < 2) return;
            int rpm = log.Index("rpm");
            int lambda = log.Index("lambda");
            if (rpm < 0) rpm = Math.Min(1, log.Headers.Length - 1);
            if (lambda < 0) lambda = rpm;
            DrawSeries(e.Graphics, rpm, Color.FromArgb(23, 33, 28));
            if (lambda != rpm) DrawSeries(e.Graphics, lambda, Color.FromArgb(12, 110, 74));
        }

        void DrawSeries(Graphics graphics, int column, Color color)
        {
            double min = log.Rows[0][column];
            double max = min;
            foreach (double[] row in log.Rows)
            {
                if (row[column] < min) min = row[column];
                if (row[column] > max) max = row[column];
            }
            if (Math.Abs(max - min) < 0.0001) max = min + 1;
            var points = new PointF[log.Rows.Count];
            for (int i = 0; i < log.Rows.Count; i++)
            {
                float x = 12 + (Width - 24) * (i / (float)(log.Rows.Count - 1));
                float y = Height - 16 - (float)((log.Rows[i][column] - min) / (max - min) * (Height - 32));
                points[i] = new PointF(x, y);
            }
            using (var pen = new Pen(color, 2f)) graphics.DrawLines(pen, points);
            using (var brush = new SolidBrush(color)) graphics.DrawString(log.Headers[column], Font, brush, 12, 8);
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

        public SurfaceMesh()
        {
            SetStyle(ControlStyles.AllPaintingInWmPaint | ControlStyles.UserPaint | ControlStyles.OptimizedDoubleBuffer, true);
            BackColor = Color.FromArgb(247, 248, 246);
        }

        public void Show(TuneTable value)
        {
            table = value;
            Invalidate();
        }

        protected override void OnPaint(PaintEventArgs e)
        {
            base.OnPaint(e);
            e.Graphics.Clear(BackColor);
            if (table == null || table.Values == null || table.Values.Length == 0 || table.Values[0].Length == 0) return;
            double min = table.Values[0][0];
            double max = min;
            for (int y = 0; y < table.Values.Length; y++)
            {
                for (int x = 0; x < table.Values[y].Length; x++)
                {
                    double value = table.Values[y][x];
                    if (value < min) min = value;
                    if (value > max) max = value;
                }
            }
            double span = max - min;
            if (span < 0.0001) span = 1;
            int rows = table.Values.Length;
            int columns = table.Values[0].Length;
            float pitchX = Math.Max(10f, (Width - 28) / (float)columns);
            float pitchY = Math.Max(8f, (Height - 36) / (float)rows);
            float barLimit = Math.Max(12f, pitchY * 2.2f);
            using (var label = new SolidBrush(Color.FromArgb(23, 33, 28)))
                e.Graphics.DrawString(table.Name, Font, label, 8, 6);
            for (int y = rows - 1; y >= 0; y--)
            {
                for (int x = 0; x < table.Values[y].Length; x++)
                {
                    double unit = (table.Values[y][x] - min) / span;
                    float height = 4f + (float)unit * barLimit;
                    float left = 10 + x * pitchX;
                    float bottom = Height - 10 - (rows - 1 - y) * (pitchY * 0.45f);
                    using (var brush = new SolidBrush(HeatColor.Deep(unit)))
                        e.Graphics.FillRectangle(brush, left, bottom - height, Math.Max(4f, pitchX - 3f), height);
                }
            }
        }
    }
}
