using System;
using System.Collections;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.Globalization;
using System.IO;
using System.Net;
using System.Text;
using System.Threading;
using System.Web.Script.Serialization;
using System.Windows.Forms;

namespace Wideband
{
    sealed class Definition
    {
        public string Id = "";
        public string Name = "";
        public string Unit = "";
        public string Mode = "";
        public string Pid = "";
        public string Formula = "";
    }

    static class Catalog
    {
        public static List<Definition> LoadLogger(string path)
        {
            var rows = new List<Definition>();
            if (!File.Exists(path)) return rows;
            Definition current = null;
            foreach (string rawLine in File.ReadAllLines(path))
            {
                string line = rawLine.Trim();
                if (line.Length == 0 || line[0] == ';' || line[0] == '#') continue;
                if (line[0] == '[' && line[line.Length - 1] == ']')
                {
                    string section = line.Substring(1, line.Length - 2).Trim();
                    if (section == "PIDs")
                    {
                        current = null;
                        continue;
                    }
                    current = new Definition();
                    current.Id = section;
                    current.Name = section;
                    rows.Add(current);
                    continue;
                }
                if (current == null) continue;
                int split = line.IndexOf('=');
                if (split <= 0) continue;
                string key = line.Substring(0, split).Trim();
                string value = Unquote(line.Substring(split + 1).Trim());
                if (key == "Name" && value.Length > 0) current.Name = value;
                else if (key == "Unit") current.Unit = value;
                else if (key == "Mode") current.Mode = value;
                else if (key == "PID") current.Pid = value;
                else if (key == "Formula") current.Formula = value;
            }
            return rows;
        }

        public static List<string> LoadProfiles(string folder)
        {
            var names = new List<string>();
            if (!Directory.Exists(folder)) return names;
            string[] files = Directory.GetFiles(folder, "*.tblp");
            Array.Sort(files, StringComparer.OrdinalIgnoreCase);
            foreach (string file in files)
            {
                try
                {
                    byte[] magic = new byte[8];
                    using (FileStream stream = File.OpenRead(file))
                    {
                        if (stream.Read(magic, 0, 8) < 8) continue;
                    }
                    if (Encoding.ASCII.GetString(magic) == "VFTBLPE1")
                        names.Add(Path.GetFileNameWithoutExtension(file));
                }
                catch (IOException)
                {
                }
                catch (UnauthorizedAccessException)
                {
                }
            }
            return names;
        }

        static string Unquote(string value)
        {
            if (value.Length >= 2 && value[0] == '"' && value[value.Length - 1] == '"')
                return value.Substring(1, value.Length - 2);
            return value;
        }
    }

    static class Cable
    {
        public static Dictionary<string, object> Check(string dllPath)
        {
            string powershell = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.System), "..", "SysWOW64", "WindowsPowerShell", "v1.0", "powershell.exe");
            powershell = Path.GetFullPath(powershell);
            string script = Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "device_check.ps1");
            if (!File.Exists(powershell))
                return Fail("32-bit PowerShell is not available, so the cable library cannot be loaded.");
            if (!File.Exists(script))
                return Fail("The cable check script is missing beside Wideband.exe.");
            var start = new ProcessStartInfo();
            start.FileName = powershell;
            start.Arguments = "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"" + script + "\" -DllPath \"" + dllPath + "\"";
            start.WorkingDirectory = Path.GetDirectoryName(dllPath);
            start.UseShellExecute = false;
            start.RedirectStandardOutput = true;
            start.RedirectStandardError = true;
            start.CreateNoWindow = true;
            start.StandardOutputEncoding = Encoding.UTF8;
            try
            {
                using (Process process = Process.Start(start))
                {
                    string output = process.StandardOutput.ReadToEnd();
                    string error = process.StandardError.ReadToEnd();
                    if (!process.WaitForExit(25000))
                    {
                        try { process.Kill(); } catch (Exception) { }
                        return Fail("The cable check timed out. Unplug the device and try again.");
                    }
                    string text = output == null ? "" : output.Trim();
                    int startBrace = text.IndexOf('{');
                    int endBrace = text.LastIndexOf('}');
                    if (startBrace < 0 || endBrace < startBrace)
                        return Fail(text.Length > 0 ? text : (error.Trim().Length > 0 ? error.Trim() : "The cable check returned no result."));
                    var serializer = new JavaScriptSerializer();
                    var payload = serializer.Deserialize<Dictionary<string, object>>(text.Substring(startBrace, endBrace - startBrace + 1));
                    if (payload == null) return Fail("The cable check returned an unexpected result.");
                    return payload;
                }
            }
            catch (Exception ex)
            {
                return Fail(ex.Message);
            }
        }

        static Dictionary<string, object> Fail(string message)
        {
            var result = new Dictionary<string, object>();
            result["ok"] = false;
            result["error"] = message;
            return result;
        }
    }

    static class Assistant
    {
        const string Model = "grok-4.7";

        public static string Answer(string question, List<Definition> rows, Dictionary<string, object> device)
        {
            string key = Environment.GetEnvironmentVariable("XAI_API_KEY");
            if (key == null || key.Trim().Length == 0)
                return Local(question, rows);
            string prompt = "You are the assistant inside Wideband, a local program on the user's computer. "
                + "Explain only the logger definitions and cable-check result in the JSON context. "
                + "Do not provide ECU programming steps, security-access procedures, VIN changes, "
                + "or ways to decrypt calibration files. If asked to write a calibration, say this "
                + "program does not write to the ECU.\n\nContext:\n"
                + ContextJson(question, rows, device)
                + "\n\nQuestion: " + question;
            try
            {
                ServicePointManager.SecurityProtocol = (SecurityProtocolType)3072;
                var request = (HttpWebRequest)WebRequest.Create("https://api.x.ai/v1/responses");
                request.Method = "POST";
                request.ContentType = "application/json";
                request.Headers["Authorization"] = "Bearer " + key.Trim();
                request.Timeout = 40000;
                byte[] body = Encoding.UTF8.GetBytes("{\"model\":\"" + Model + "\",\"input\":" + JsonString(prompt) + "}");
                request.ContentLength = body.Length;
                using (Stream stream = request.GetRequestStream()) stream.Write(body, 0, body.Length);
                using (var response = (HttpWebResponse)request.GetResponse())
                using (var reader = new StreamReader(response.GetResponseStream()))
                    return ReadGrok(reader.ReadToEnd());
            }
            catch (WebException ex)
            {
                string detail = "";
                if (ex.Response != null)
                {
                    using (var reader = new StreamReader(ex.Response.GetResponseStream()))
                        detail = reader.ReadToEnd();
                }
                if (detail.Length > 240) detail = detail.Substring(0, 240);
                return "Grok could not answer. " + (detail.Length > 0 ? detail : ex.Message);
            }
        }

        static string Local(string question, List<Definition> rows)
        {
            List<Definition> found = Match(question, rows, 6);
            if (found.Count == 0) return "No logger definitions are loaded.";
            var text = new StringBuilder();
            text.Append("I can answer from the logger file on this computer. ");
            foreach (Definition row in found)
            {
                text.Append(row.Name);
                if (row.Unit.Length > 0) text.Append(" ").Append(row.Unit);
                text.Append(", mode ").Append(row.Mode.Length > 0 ? row.Mode : "-");
                text.Append(" PID ").Append(row.Pid.Length > 0 ? row.Pid : "-").Append(". ");
                if (row.Formula.Length > 0) text.Append("Formula ").Append(row.Formula).Append(". ");
            }
            text.Append("Set XAI_API_KEY before opening Wideband to ask Grok about these definitions. This program does not write a calibration.");
            return text.ToString();
        }

        public static List<Definition> Match(string question, List<Definition> rows, int limit)
        {
            string[] parts = question.ToLowerInvariant().Split(new char[] { ' ', '\t', '\r', '\n' }, StringSplitOptions.RemoveEmptyEntries);
            var needles = new List<string>();
            foreach (string part in parts)
                if (part.Length > 2) needles.Add(part);
            var scored = new List<KeyValuePair<int, Definition>>();
            foreach (Definition row in rows)
            {
                string hay = (row.Name + " " + row.Id + " " + row.Unit).ToLowerInvariant();
                int score = 0;
                foreach (string needle in needles)
                    if (hay.IndexOf(needle, StringComparison.Ordinal) >= 0) score++;
                if (score > 0) scored.Add(new KeyValuePair<int, Definition>(score, row));
            }
            scored.Sort(delegate (KeyValuePair<int, Definition> a, KeyValuePair<int, Definition> b)
            {
                int byScore = b.Key.CompareTo(a.Key);
                if (byScore != 0) return byScore;
                return string.Compare(a.Value.Name, b.Value.Name, StringComparison.OrdinalIgnoreCase);
            });
            var result = new List<Definition>();
            int count = Math.Min(limit, scored.Count);
            for (int i = 0; i < count; i++) result.Add(scored[i].Value);
            if (result.Count == 0)
            {
                int fallback = Math.Min(3, rows.Count);
                for (int i = 0; i < fallback; i++) result.Add(rows[i]);
            }
            return result;
        }

        static string ContextJson(string question, List<Definition> rows, Dictionary<string, object> device)
        {
            var serializer = new JavaScriptSerializer();
            var definitions = new List<Dictionary<string, string>>();
            foreach (Definition row in Match(question, rows, 6))
            {
                var item = new Dictionary<string, string>();
                item["name"] = row.Name;
                item["unit"] = row.Unit;
                item["mode"] = row.Mode;
                item["pid"] = row.Pid;
                item["formula"] = row.Formula;
                definitions.Add(item);
            }
            var context = new Dictionary<string, object>();
            context["definitions"] = definitions;
            context["device"] = device ?? (object)"The cable has not been checked in this session.";
            return serializer.Serialize(context);
        }

        static string JsonString(string value)
        {
            var serializer = new JavaScriptSerializer();
            return serializer.Serialize(value);
        }

        static string ReadGrok(string json)
        {
            var serializer = new JavaScriptSerializer();
            var root = serializer.Deserialize<Dictionary<string, object>>(json);
            if (root == null) return "Grok returned an empty answer.";
            object direct;
            if (root.TryGetValue("output_text", out direct) && direct != null && direct.ToString().Trim().Length > 0)
                return direct.ToString().Trim();
            object outputObj;
            if (!root.TryGetValue("output", out outputObj) || outputObj == null)
                return "Grok returned an empty answer.";
            var output = outputObj as ArrayList;
            var texts = new StringBuilder();
            if (output != null)
            {
                foreach (object itemObj in output)
                {
                    var item = itemObj as Dictionary<string, object>;
                    if (item == null) continue;
                    object contentObj;
                    if (!item.TryGetValue("content", out contentObj)) continue;
                    var content = contentObj as ArrayList;
                    if (content == null) continue;
                    foreach (object partObj in content)
                    {
                        var part = partObj as Dictionary<string, object>;
                        if (part == null) continue;
                        object type;
                        object text;
                        part.TryGetValue("type", out type);
                        part.TryGetValue("text", out text);
                        if (type != null && type.ToString() == "output_text" && text != null)
                            texts.AppendLine(text.ToString());
                    }
                }
            }
            string result = texts.ToString().Trim();
            return result.Length > 0 ? result : "Grok returned an empty answer.";
        }
    }

    sealed class MainForm : Form
    {
        readonly List<Definition> definitions;
        readonly string installPath;
        readonly string dllPath;
        Dictionary<string, object> device;
        readonly Font ui;
        readonly Font titleFont;
        readonly Color ink = Color.FromArgb(23, 33, 28);
        readonly Color muted = Color.FromArgb(61, 74, 68);
        readonly Color line = Color.FromArgb(195, 202, 196);
        readonly Color panel = Color.FromArgb(247, 248, 246);
        readonly Color accent = Color.FromArgb(12, 110, 74);
        readonly Color tint = Color.FromArgb(229, 242, 235);

        readonly Button calibrateButton;
        readonly Button logButton;
        readonly Button cableButton;
        readonly Button loggerButton;
        readonly Button profilesButton;
        readonly CalibratePanel calibrateView;
        readonly LogPanel logView;
        readonly Panel cableView;
        readonly Panel loggerView;
        readonly Panel profileView;
        readonly Label summary;
        readonly Label cableStatus;
        readonly Label linkBadge;
        readonly Button connectButton;
        readonly Button readButton;
        readonly Button transferButton;
        readonly Button disconnectButton;
        readonly ComboBox platformBox;
        readonly ComboBox protocolBox;
        readonly TextBox addressBox;
        readonly TextBox lengthBox;
        readonly ComboBox chunkBox;
        readonly TextBox cableLog;
        readonly ProgressBar transferProgress;
        bool transferring;
        bool cableBusy;
        bool suppressProfile;
        readonly TextBox findBox;
        readonly ListView definitionList;
        readonly Label detail;
        readonly ListBox profileList;
        readonly TextBox threadBox;
        readonly TextBox questionBox;
        readonly Button askButton;

        public MainForm(string installPath, List<Definition> definitions, List<string> profiles)
        {
            this.installPath = installPath;
            this.definitions = definitions;
            this.dllPath = Path.Combine(installPath, "esp32.dll");
            ui = new Font("Bahnschrift", 11f, FontStyle.Regular, GraphicsUnit.Point);
            if (ui.Name != "Bahnschrift")
            {
                ui.Dispose();
                ui = new Font("Segoe UI", 11f, FontStyle.Regular, GraphicsUnit.Point);
            }
            titleFont = new Font(ui.FontFamily, 18f, FontStyle.Regular, GraphicsUnit.Point);

            Text = "Wideband";
            Font = ui;
            BackColor = Color.FromArgb(223, 227, 222);
            ForeColor = ink;
            MinimumSize = new Size(980, 680);
            Size = new Size(1180, 760);
            StartPosition = FormStartPosition.CenterScreen;

            var header = new Panel();
            header.Dock = DockStyle.Top;
            header.Height = 72;
            header.BackColor = BackColor;
            var nameLabel = new Label();
            nameLabel.Dock = DockStyle.Top;
            nameLabel.Height = 36;
            nameLabel.Padding = new Padding(20, 12, 20, 0);
            nameLabel.Font = titleFont;
            nameLabel.ForeColor = ink;
            nameLabel.Text = "Wideband";
            summary = new Label();
            summary.Dock = DockStyle.Fill;
            summary.Padding = new Padding(20, 0, 20, 0);
            summary.ForeColor = muted;
            summary.Text = "Sample calibration, " + definitions.Count + " logger definitions, " + profiles.Count + " profiles.";
            header.Controls.Add(summary);
            header.Controls.Add(nameLabel);

            var status = new Panel();
            status.Dock = DockStyle.Bottom;
            status.Height = 36;
            status.BackColor = BackColor;
            var pathLabel = new Label();
            pathLabel.Dock = DockStyle.Fill;
            pathLabel.TextAlign = ContentAlignment.MiddleLeft;
            pathLabel.Padding = new Padding(16, 0, 0, 0);
            pathLabel.ForeColor = muted;
            pathLabel.Text = installPath;
            var limitLabel = new Label();
            limitLabel.Dock = DockStyle.Right;
            limitLabel.Width = 220;
            limitLabel.TextAlign = ContentAlignment.MiddleRight;
            limitLabel.Padding = new Padding(0, 0, 16, 0);
            limitLabel.ForeColor = muted;
            limitLabel.Text = "Asks before a read or a write";
            status.Controls.Add(pathLabel);
            status.Controls.Add(limitLabel);

            var nav = new Panel();
            nav.Dock = DockStyle.Left;
            nav.Width = 168;
            nav.Padding = new Padding(12);
            nav.BackColor = BackColor;
            calibrateButton = NavButton("Calibrate");
            logButton = NavButton("Log");
            cableButton = NavButton("Cable");
            loggerButton = NavButton("Logger");
            profilesButton = NavButton("Profiles");
            calibrateButton.Click += delegate { ShowView("calibrate"); };
            logButton.Click += delegate { ShowView("log"); };
            cableButton.Click += delegate { ShowView("cable"); };
            loggerButton.Click += delegate { ShowView("logger"); };
            profilesButton.Click += delegate { ShowView("profiles"); };
            var navStack = new FlowLayoutPanel();
            navStack.Dock = DockStyle.Top;
            navStack.FlowDirection = FlowDirection.TopDown;
            navStack.WrapContents = false;
            navStack.AutoSize = true;
            navStack.Controls.Add(calibrateButton);
            navStack.Controls.Add(logButton);
            navStack.Controls.Add(loggerButton);
            navStack.Controls.Add(cableButton);
            navStack.Controls.Add(profilesButton);
            nav.Controls.Add(navStack);

            var assistant = new Panel();
            assistant.Dock = DockStyle.Right;
            assistant.Width = 320;
            assistant.Padding = new Padding(12);
            assistant.BackColor = BackColor;
            var assistantTitle = new Label();
            assistantTitle.Dock = DockStyle.Top;
            assistantTitle.Height = 28;
            assistantTitle.Text = "Assistant";
            assistantTitle.Font = new Font(ui, FontStyle.Bold);
            threadBox = new TextBox();
            threadBox.Multiline = true;
            threadBox.ReadOnly = true;
            threadBox.ScrollBars = ScrollBars.Vertical;
            threadBox.Dock = DockStyle.Fill;
            threadBox.BorderStyle = BorderStyle.None;
            threadBox.BackColor = panel;
            threadBox.AccessibleName = "Assistant transcript";
            threadBox.AccessibleRole = AccessibleRole.Text;
            questionBox = new TextBox();
            questionBox.Multiline = true;
            questionBox.Dock = DockStyle.Bottom;
            questionBox.Height = 72;
            questionBox.AccessibleName = "Ask about a definition";
            questionBox.AccessibleRole = AccessibleRole.Text;
            askButton = ActionButton("Ask");
            askButton.Dock = DockStyle.Bottom;
            askButton.Click += AskClicked;
            assistant.Controls.Add(threadBox);
            assistant.Controls.Add(questionBox);
            assistant.Controls.Add(askButton);
            assistant.Controls.Add(assistantTitle);

            var content = new Panel();
            content.Dock = DockStyle.Fill;
            content.BackColor = panel;
            content.Padding = new Padding(16);

            cableView = new Panel();
            cableView.Dock = DockStyle.Fill;
            cableView.BackColor = panel;
            var cableTitle = SectionTitle("Cable");
            var cableCopy = new Label();
            cableCopy.Dock = DockStyle.Top;
            cableCopy.Height = 120;
            cableCopy.Text = "Connect Cable opens the J2534 device, reads the firmware version, and leaves the channel open. Tester present (0x3E) is sent every 2 seconds. Tacoma 2009+ and Silverado 2007+ use ISO 15765 on CAN at 500 kbps. Tacoma 2005-2008 uses ISO 14230 K-Line at 10400 baud. Silverado 1999-2007 uses J1850 VPW at 10400 baud. Read ECU uses service 0x23. Write sends session 0x10, security 0x27, download 0x34, transfer 0x36, exit 0x37, and reset 0x11 after confirmation. No security key and no checksum are built in.";
            platformBox = new ComboBox();
            platformBox.DropDownStyle = ComboBoxStyle.DropDownList;
            platformBox.Width = 180;
            platformBox.Items.Add("Toyota Tacoma");
            platformBox.Items.Add("Chevrolet Silverado");
            platformBox.AccessibleName = "Vehicle platform";
            protocolBox = new ComboBox();
            protocolBox.DropDownStyle = ComboBoxStyle.DropDownList;
            protocolBox.Width = 280;
            protocolBox.AccessibleName = "Diagnostic protocol";
            linkBadge = new Label();
            linkBadge.Text = "CLOSED";
            linkBadge.AutoSize = true;
            linkBadge.Padding = new Padding(8, 6, 8, 6);
            linkBadge.BackColor = Color.FromArgb(223, 227, 222);
            linkBadge.TextAlign = ContentAlignment.MiddleCenter;
            var addressLabel = new Label();
            addressLabel.Text = "Address";
            addressLabel.AutoSize = true;
            addressLabel.Padding = new Padding(8, 8, 0, 0);
            addressBox = new TextBox();
            addressBox.Width = 110;
            addressBox.AccessibleName = "Memory address";
            addressBox.AccessibleRole = AccessibleRole.Text;
            var lengthLabel = new Label();
            lengthLabel.Text = "Length";
            lengthLabel.AutoSize = true;
            lengthLabel.Padding = new Padding(8, 8, 0, 0);
            lengthBox = new TextBox();
            lengthBox.Width = 90;
            lengthBox.AccessibleName = "Read length";
            lengthBox.AccessibleRole = AccessibleRole.Text;
            chunkBox = new ComboBox();
            chunkBox.DropDownStyle = ComboBoxStyle.DropDownList;
            chunkBox.Width = 80;
            chunkBox.AccessibleName = "Transfer chunk";
            connectButton = ActionButton("Connect Cable");
            connectButton.AutoSize = true;
            connectButton.Click += ConnectClicked;
            readButton = ActionButton("Read ECU");
            readButton.AutoSize = true;
            readButton.Enabled = false;
            readButton.Click += ReadClicked;
            transferButton = ActionButton("Write/Flash Calibration");
            transferButton.AutoSize = true;
            transferButton.Enabled = false;
            transferButton.Click += TransferClicked;
            disconnectButton = ActionButton("Disconnect");
            disconnectButton.AutoSize = true;
            disconnectButton.Enabled = false;
            disconnectButton.Click += DisconnectClicked;
            var cableTools = new FlowLayoutPanel();
            cableTools.Dock = DockStyle.Top;
            cableTools.Height = 148;
            cableTools.WrapContents = true;
            cableTools.Controls.Add(platformBox);
            cableTools.Controls.Add(protocolBox);
            cableTools.Controls.Add(linkBadge);
            cableTools.Controls.Add(addressLabel);
            cableTools.Controls.Add(addressBox);
            cableTools.Controls.Add(lengthLabel);
            cableTools.Controls.Add(lengthBox);
            cableTools.Controls.Add(chunkBox);
            cableTools.Controls.Add(connectButton);
            cableTools.Controls.Add(readButton);
            cableTools.Controls.Add(transferButton);
            cableTools.Controls.Add(disconnectButton);
            cableStatus = new Label();
            cableStatus.Dock = DockStyle.Top;
            cableStatus.Height = 72;
            cableStatus.Padding = new Padding(8);
            cableStatus.BackColor = Color.FromArgb(223, 227, 222);
            cableStatus.Text = "The channel is closed.";
            var logCaption = new Label();
            logCaption.Text = "Communication log";
            logCaption.Dock = DockStyle.Top;
            logCaption.Height = 22;
            logCaption.TextAlign = ContentAlignment.MiddleLeft;
            cableLog = new TextBox();
            cableLog.Multiline = true;
            cableLog.ReadOnly = true;
            cableLog.ScrollBars = ScrollBars.Vertical;
            cableLog.Dock = DockStyle.Fill;
            cableLog.Font = new Font("Consolas", 9f);
            cableLog.BackColor = panel;
            cableLog.AccessibleName = "Communication log";
            cableLog.AccessibleRole = AccessibleRole.Text;
            transferProgress = new ProgressBar();
            transferProgress.Dock = DockStyle.Bottom;
            transferProgress.Height = 22;
            transferProgress.Maximum = 100;
            transferProgress.AccessibleName = "Transfer progress";
            var progressLabel = new Label();
            progressLabel.Text = "Transfer progress";
            progressLabel.Dock = DockStyle.Bottom;
            progressLabel.Height = 22;
            progressLabel.TextAlign = ContentAlignment.MiddleLeft;
            platformBox.SelectedIndexChanged += PlatformChanged;
            protocolBox.SelectedIndexChanged += ProfileChanged;
            suppressProfile = true;
            platformBox.SelectedIndex = 0;
            suppressProfile = false;
            FillProtocols();
            VehicleLink.Current.Noted += HoldNoted;
            VehicleLink.Current.Traced += CableTraced;
            cableView.Controls.Add(cableLog);
            cableView.Controls.Add(progressLabel);
            cableView.Controls.Add(transferProgress);
            cableView.Controls.Add(logCaption);
            cableView.Controls.Add(cableStatus);
            cableView.Controls.Add(cableTools);
            cableView.Controls.Add(cableCopy);
            cableView.Controls.Add(cableTitle);

            loggerView = new Panel();
            loggerView.Dock = DockStyle.Fill;
            loggerView.BackColor = panel;
            var loggerTitle = SectionTitle("Logger");
            findBox = new TextBox();
            findBox.Dock = DockStyle.Top;
            findBox.AccessibleName = "Find";
            findBox.AccessibleRole = AccessibleRole.Text;
            findBox.TextChanged += delegate { FillDefinitions(findBox.Text); };
            var findLabel = new Label();
            findLabel.Text = "Find";
            findLabel.Dock = DockStyle.Top;
            findLabel.Height = 22;
            findLabel.ForeColor = muted;
            definitionList = new ListView();
            definitionList.Dock = DockStyle.Fill;
            definitionList.View = View.Details;
            definitionList.FullRowSelect = true;
            definitionList.HeaderStyle = ColumnHeaderStyle.Nonclickable;
            definitionList.BorderStyle = BorderStyle.None;
            definitionList.BackColor = panel;
            definitionList.Columns.Add("Name", 360);
            definitionList.Columns.Add("Unit", 80);
            definitionList.SelectedIndexChanged += DefinitionSelected;
            detail = new Label();
            detail.Dock = DockStyle.Bottom;
            detail.Height = 72;
            detail.Padding = new Padding(8);
            detail.BackColor = Color.FromArgb(223, 227, 222);
            detail.Text = "Choose a definition.";
            loggerView.Controls.Add(definitionList);
            loggerView.Controls.Add(detail);
            loggerView.Controls.Add(findBox);
            loggerView.Controls.Add(findLabel);
            loggerView.Controls.Add(loggerTitle);

            profileView = new Panel();
            profileView.Dock = DockStyle.Fill;
            profileView.BackColor = panel;
            var profileTitle = SectionTitle("Profiles");
            var profileCopy = new Label();
            profileCopy.Dock = DockStyle.Top;
            profileCopy.Height = 40;
            profileCopy.ForeColor = muted;
            profileCopy.Text = "These are the profile filenames from the install. The file bodies stay opaque.";
            profileList = new ListBox();
            profileList.Dock = DockStyle.Fill;
            profileList.BorderStyle = BorderStyle.None;
            profileList.BackColor = panel;
            profileList.IntegralHeight = false;
            foreach (string name in profiles) profileList.Items.Add(name);
            if (profiles.Count == 0) profileList.Items.Add("No profiles with a VFTBLPE1 header were found.");
            profileView.Controls.Add(profileList);
            profileView.Controls.Add(profileCopy);
            profileView.Controls.Add(profileTitle);

            string definitionPath = Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "sample-definition.json");
            TuneDocument tune = TuneDocument.LoadOrCreate(definitionPath);
            calibrateView = new CalibratePanel(tune, definitionPath);
            calibrateView.Dock = DockStyle.Fill;
            logView = new LogPanel(tune, calibrateView, definitionPath);
            logView.Dock = DockStyle.Fill;
            content.Controls.Add(profileView);
            content.Controls.Add(loggerView);
            content.Controls.Add(cableView);
            content.Controls.Add(logView);
            content.Controls.Add(calibrateView);

            Controls.Add(content);
            Controls.Add(assistant);
            Controls.Add(nav);
            Controls.Add(status);
            Controls.Add(header);

            FillDefinitions("");
            ShowView("calibrate");
        }

        Button NavButton(string text)
        {
            var button = new Button();
            button.Text = text;
            button.Width = 140;
            button.Height = 36;
            button.Margin = new Padding(0, 0, 0, 6);
            button.FlatStyle = FlatStyle.Flat;
            button.FlatAppearance.BorderSize = 0;
            button.TextAlign = ContentAlignment.MiddleLeft;
            button.Padding = new Padding(10, 0, 0, 0);
            button.Cursor = Cursors.Hand;
            button.AccessibleRole = AccessibleRole.PushButton;
            return button;
        }

        Button ActionButton(string text)
        {
            var button = new Button();
            button.Text = text;
            button.Height = 36;
            button.FlatStyle = FlatStyle.Flat;
            button.FlatAppearance.BorderSize = 0;
            button.BackColor = accent;
            button.ForeColor = Color.FromArgb(244, 250, 247);
            button.Cursor = Cursors.Hand;
            button.AccessibleRole = AccessibleRole.PushButton;
            return button;
        }

        Label SectionTitle(string text)
        {
            var label = new Label();
            label.Dock = DockStyle.Top;
            label.Height = 32;
            label.Font = new Font(ui, FontStyle.Bold);
            label.Text = text;
            return label;
        }

        void ShowView(string name)
        {
            calibrateView.Visible = name == "calibrate";
            logView.Visible = name == "log";
            cableView.Visible = name == "cable";
            loggerView.Visible = name == "logger";
            profileView.Visible = name == "profiles";
            MarkNav(calibrateButton, name == "calibrate");
            MarkNav(logButton, name == "log");
            MarkNav(cableButton, name == "cable");
            MarkNav(loggerButton, name == "logger");
            MarkNav(profilesButton, name == "profiles");
        }

        void MarkNav(Button button, bool selected)
        {
            button.BackColor = selected ? accent : BackColor;
            button.ForeColor = selected ? Color.FromArgb(244, 250, 247) : ink;
        }

        void FillDefinitions(string query)
        {
            string needle = query.Trim().ToLowerInvariant();
            definitionList.BeginUpdate();
            definitionList.Items.Clear();
            foreach (Definition row in definitions)
            {
                if (needle.Length > 0
                    && row.Name.ToLowerInvariant().IndexOf(needle, StringComparison.Ordinal) < 0
                    && row.Id.ToLowerInvariant().IndexOf(needle, StringComparison.Ordinal) < 0)
                    continue;
                var item = new ListViewItem(row.Name);
                item.SubItems.Add(row.Unit);
                item.Tag = row;
                definitionList.Items.Add(item);
            }
            definitionList.EndUpdate();
            if (definitionList.Items.Count == 0)
                detail.Text = "No definitions match this search.";
            else
            {
                definitionList.Items[0].Selected = true;
                DefinitionSelected(null, EventArgs.Empty);
            }
        }

        void DefinitionSelected(object sender, EventArgs e)
        {
            if (definitionList.SelectedItems.Count == 0)
            {
                detail.Text = "Choose a definition.";
                return;
            }
            var row = definitionList.SelectedItems[0].Tag as Definition;
            if (row == null) return;
            detail.Text = row.Name + " is mode " + (row.Mode.Length > 0 ? row.Mode : "-")
                + " PID " + (row.Pid.Length > 0 ? row.Pid : "-")
                + (row.Unit.Length > 0 ? ", unit " + row.Unit : "")
                + (row.Formula.Length > 0 ? ". Formula " + row.Formula + "." : ".");
        }

        void HoldNoted(string text)
        {
            if (IsDisposed || transferring) return;
            try
            {
                BeginInvoke((Action)delegate
                {
                    if (IsDisposed || transferring || !VehicleLink.Current.IsConnected) return;
                    cableStatus.Text = text;
                });
            }
            catch (Exception)
            {
            }
        }

        void CableTraced(string text)
        {
            if (IsDisposed) return;
            try
            {
                BeginInvoke((Action)delegate
                {
                    if (!IsDisposed) AppendLog(text);
                });
            }
            catch (Exception)
            {
            }
        }

        void PlatformChanged(object sender, EventArgs e)
        {
            if (suppressProfile) return;
            FillProtocols();
            if (VehicleLink.Current.IsConnected && !transferring && !cableBusy)
                ConnectClicked(sender, e);
        }

        void ProfileChanged(object sender, EventArgs e)
        {
            if (suppressProfile) return;
            FillChunks();
            if (transferring || cableBusy) return;
            if (VehicleLink.Current.IsConnected)
            {
                AppendLog("Switching to " + SelectedProfile().Name + ".");
                ConnectClicked(sender, e);
                return;
            }
            cableStatus.Text = SelectedProfile().Name + ". The channel is closed.";
        }

        void FillProtocols()
        {
            suppressProfile = true;
            protocolBox.Items.Clear();
            if (platformBox.SelectedIndex == 1)
            {
                protocolBox.Items.Add("2007+ CAN 500 kbps (ISO 15765)");
                protocolBox.Items.Add("1999-2007 J1850 VPW");
            }
            else
            {
                protocolBox.Items.Add("2009+ CAN 500 kbps (ISO 15765)");
                protocolBox.Items.Add("2005-2008 K-Line (ISO 14230)");
            }
            protocolBox.SelectedIndex = 0;
            suppressProfile = false;
            FillChunks();
            if (cableStatus != null && !VehicleLink.Current.IsConnected && !transferring)
                cableStatus.Text = SelectedProfile().Name + ". The channel is closed.";
        }

        void FillChunks()
        {
            EcuProfile profile = SelectedProfile();
            int[] chunks = profile.MaxChunk >= 1024 ? new int[] { 256, 1024 }
                : profile.MaxChunk >= 128 ? new int[] { 64, 128 }
                : new int[] { 4, 6 };
            chunkBox.Items.Clear();
            for (int i = 0; i < chunks.Length; i++)
            {
                if (chunks[i] <= profile.MaxChunk)
                    chunkBox.Items.Add(chunks[i].ToString(CultureInfo.InvariantCulture));
            }
            if (chunkBox.Items.Count == 0)
                chunkBox.Items.Add(profile.MaxChunk.ToString(CultureInfo.InvariantCulture));
            chunkBox.SelectedIndex = 0;
        }

        EcuProfile SelectedProfile()
        {
            bool silverado = platformBox.SelectedIndex == 1;
            bool second = protocolBox.SelectedIndex == 1;
            if (silverado) return second ? EcuProfile.SilveradoVpw() : EcuProfile.SilveradoCan();
            return second ? EcuProfile.TacomaKLine() : EcuProfile.TacomaCan();
        }

        int SelectedChunk()
        {
            int chunk;
            if (chunkBox.SelectedItem == null || !int.TryParse(chunkBox.SelectedItem.ToString(), NumberStyles.Integer, CultureInfo.InvariantCulture, out chunk))
                return 0;
            return chunk;
        }

        bool TryAddress(out long address)
        {
            address = 0;
            string hex = addressBox.Text.Trim();
            if (hex.StartsWith("0x") || hex.StartsWith("0X")) hex = hex.Substring(2);
            if (hex.Length == 0 || !long.TryParse(hex, NumberStyles.HexNumber, CultureInfo.InvariantCulture, out address))
            {
                cableStatus.Text = "Enter the memory address in hexadecimal.";
                return false;
            }
            return true;
        }

        void AppendLog(string line)
        {
            if (line == null || line.Length == 0 || cableLog == null) return;
            cableLog.AppendText(line + Environment.NewLine);
            string[] lines = cableLog.Lines;
            if (lines.Length <= 400) return;
            var kept = new string[300];
            Array.Copy(lines, lines.Length - 300, kept, 0, 300);
            cableLog.Lines = kept;
            cableLog.SelectionStart = cableLog.TextLength;
            cableLog.ScrollToCaret();
        }

        void ShowProgress(FlashProgress update)
        {
            if (update == null) return;
            int percent = update.Percent;
            if (percent < 0) percent = 0;
            if (percent > 100) percent = 100;
            transferProgress.Value = percent;
            string line = update.Stage.ToString() + " " + percent.ToString(CultureInfo.InvariantCulture)
                + "%  " + update.KilobytesPerSecond.ToString("0.0", CultureInfo.InvariantCulture)
                + " KB/s. " + update.Detail;
            cableStatus.Text = line;
            AppendLog(line);
        }

        void SetCableButtons(bool open)
        {
            connectButton.Enabled = !open;
            disconnectButton.Enabled = open;
            readButton.Enabled = open;
            transferButton.Enabled = open;
            linkBadge.Text = open ? "OPEN" : "CLOSED";
            linkBadge.BackColor = open ? accent : Color.FromArgb(223, 227, 222);
            linkBadge.ForeColor = open ? Color.White : ink;
        }

        void ConnectClicked(object sender, EventArgs e)
        {
            if (cableBusy || transferring) return;
            cableBusy = true;
            connectButton.Enabled = false;
            readButton.Enabled = false;
            transferButton.Enabled = false;
            disconnectButton.Enabled = false;
            cableStatus.BackColor = Color.FromArgb(223, 227, 222);
            cableStatus.Text = "Opening the cable and holding the channel.";
            EcuProfile profile = SelectedProfile();
            string library = dllPath;
            ThreadPool.QueueUserWorkItem(delegate
            {
                bool ok = false;
                string text;
                try
                {
                    var settings = new CanChannelSettings();
                    settings.J2534LibraryPath = library;
                    VehicleLink.Current.ConnectAndHold(settings, profile);
                    ok = true;
                    text = "Channel held. " + profile.Name + ". Device "
                        + VehicleLink.Current.DeviceId.ToString()
                        + ", channel " + VehicleLink.Current.ChannelId.ToString() + ".";
                    if (VehicleLink.Current.Firmware.Length > 0)
                        text += " Firmware " + VehicleLink.Current.Firmware + ".";
                    text += " Tester present runs every 2 seconds.";
                    var snapshot = new Dictionary<string, object>();
                    snapshot["ok"] = true;
                    snapshot["firmware"] = VehicleLink.Current.Firmware;
                    snapshot["device_id"] = VehicleLink.Current.DeviceId.ToString();
                    snapshot["channel_id"] = VehicleLink.Current.ChannelId.ToString();
                    snapshot["profile"] = profile.Id;
                    device = snapshot;
                }
                catch (Exception ex)
                {
                    text = ex.Message;
                    var snapshot = new Dictionary<string, object>();
                    snapshot["ok"] = false;
                    snapshot["error"] = ex.Message;
                    device = snapshot;
                }
                BeginInvoke((Action)delegate
                {
                    cableBusy = false;
                    cableStatus.BackColor = ok ? tint : Color.FromArgb(243, 228, 225);
                    cableStatus.Text = text.Length > 0 ? text : "The cable did not open.";
                    AppendLog(cableStatus.Text);
                    SetCableButtons(ok);
                    if (!ok) transferProgress.Value = 0;
                });
            });
        }

        void ReadClicked(object sender, EventArgs e)
        {
            if (!VehicleLink.Current.IsConnected)
            {
                cableStatus.Text = "Connect the cable first.";
                return;
            }
            long address;
            if (!TryAddress(out address)) return;
            int length;
            string hex = lengthBox.Text.Trim();
            if (hex.StartsWith("0x") || hex.StartsWith("0X")) hex = hex.Substring(2);
            if (hex.Length == 0 || !int.TryParse(hex, NumberStyles.HexNumber, CultureInfo.InvariantCulture, out length) || length < 1)
            {
                cableStatus.Text = "Enter the read length in hexadecimal.";
                return;
            }
            if (length > 8 * 1024 * 1024)
            {
                cableStatus.Text = "The read is larger than 8 MB.";
                return;
            }
            int chunk = SelectedChunk();
            EcuProfile profile = SelectedProfile();
            DialogResult confirm = MessageBox.Show(this,
                "Read " + length.ToString(CultureInfo.InvariantCulture) + " bytes from address 0x" + address.ToString("X")
                + " on " + profile.Name + "? This sends diagnostic session 0x" + profile.ReadSession.ToString("X2")
                + ", security access, and service 0x23. It does not erase, write, or reset.",
                "Read ECU",
                MessageBoxButtons.OKCancel,
                MessageBoxIcon.Warning);
            if (confirm != DialogResult.OK) return;
            using (var dialog = new SaveFileDialog())
            {
                dialog.Title = "Save the ECU read";
                dialog.Filter = "Binary files (*.bin)|*.bin|All files (*.*)|*.*";
                dialog.FileName = "ecu-read.bin";
                if (dialog.ShowDialog(this) != DialogResult.OK) return;
                string path = dialog.FileName;
                transferring = true;
                connectButton.Enabled = false;
                readButton.Enabled = false;
                transferButton.Enabled = false;
                transferProgress.Value = 0;
                cableStatus.BackColor = Color.FromArgb(223, 227, 222);
                cableStatus.Text = "Starting the read.";
                AppendLog("Read started at 0x" + address.ToString("X") + ".");
                ThreadPool.QueueUserWorkItem(delegate
                {
                    string error = "";
                    bool ok = false;
                    try
                    {
                        byte[] data = VehicleLink.Current.ReadCalibration(address, length, chunk, delegate (FlashProgress update)
                        {
                            try
                            {
                                BeginInvoke((Action)delegate { if (!IsDisposed) ShowProgress(update); });
                            }
                            catch (Exception) { }
                        });
                        File.WriteAllBytes(path, data);
                        ok = true;
                    }
                    catch (Exception ex)
                    {
                        error = ex.Message;
                    }
                    BeginInvoke((Action)delegate
                    {
                        transferring = false;
                        if (ok) transferProgress.Value = 100;
                        cableStatus.BackColor = ok ? tint : Color.FromArgb(243, 228, 225);
                        cableStatus.Text = ok
                            ? "Read finished. The channel is still open."
                            : (error.Length == 0 ? "The read stopped." : error);
                        AppendLog(cableStatus.Text);
                        SetCableButtons(VehicleLink.Current.IsConnected);
                    });
                });
            }
        }

        void TransferClicked(object sender, EventArgs e)
        {
            if (!VehicleLink.Current.IsConnected)
            {
                cableStatus.Text = "Connect the cable first.";
                return;
            }
            using (var dialog = new OpenFileDialog())
            {
                dialog.Title = "Calibration binary to write";
                dialog.Filter = "Binary files (*.bin)|*.bin|All files (*.*)|*.*";
                if (dialog.ShowDialog(this) != DialogResult.OK) return;
                long address;
                if (!TryAddress(out address)) return;
                byte[] image;
                try
                {
                    if (dialog.FileName.EndsWith(".enc", StringComparison.OrdinalIgnoreCase))
                    {
                        cableStatus.Text = "Encrypted definition files are not transferred.";
                        return;
                    }
                    image = File.ReadAllBytes(dialog.FileName);
                    if (image.Length >= 8 && Encoding.ASCII.GetString(image, 0, 8) == "VFTBLPE1")
                    {
                        cableStatus.Text = "This profile body is opaque and is not transferred.";
                        return;
                    }
                }
                catch (Exception ex)
                {
                    cableStatus.Text = ex.Message;
                    return;
                }
                if (image.Length == 0)
                {
                    cableStatus.Text = "The file is empty.";
                    return;
                }
                if (image.Length > 8 * 1024 * 1024)
                {
                    cableStatus.Text = "The file is larger than 8 MB.";
                    return;
                }
                int chunk = SelectedChunk();
                EcuProfile profile = SelectedProfile();
                DialogResult confirm = MessageBox.Show(this,
                    "Write " + image.Length.ToString(CultureInfo.InvariantCulture) + " bytes to address 0x" + address.ToString("X")
                    + " on " + profile.Name + "? This sends session 0x" + profile.ProgrammingSession.ToString("X2")
                    + ", security access, download 0x34, transfer 0x36, exit 0x37, and reset 0x11. No security key is built in. No checksum is applied.",
                    "Write calibration",
                    MessageBoxButtons.OKCancel,
                    MessageBoxIcon.Warning);
                if (confirm != DialogResult.OK) return;
                transferring = true;
                connectButton.Enabled = false;
                readButton.Enabled = false;
                transferButton.Enabled = false;
                transferProgress.Value = 0;
                cableStatus.BackColor = Color.FromArgb(223, 227, 222);
                cableStatus.Text = "Starting the transfer.";
                AppendLog("Write started at 0x" + address.ToString("X") + ".");
                ThreadPool.QueueUserWorkItem(delegate
                {
                    FlashResult result;
                    try
                    {
                        result = VehicleLink.Current.Transfer(address, image, chunk, delegate (FlashProgress update)
                        {
                            try
                            {
                                BeginInvoke((Action)delegate { if (!IsDisposed) ShowProgress(update); });
                            }
                            catch (Exception) { }
                        });
                    }
                    catch (Exception ex)
                    {
                        result = new FlashResult();
                        result.Error = ex.Message;
                    }
                    BeginInvoke((Action)delegate
                    {
                        transferring = false;
                        bool ok = result != null && result.Complete;
                        if (ok) transferProgress.Value = 100;
                        cableStatus.BackColor = ok ? tint : Color.FromArgb(243, 228, 225);
                        cableStatus.Text = ok
                            ? "Transfer finished. The ECU was reset. The cable channel is still open. No checksum function is configured."
                            : (result == null || result.Error.Length == 0 ? "The transfer stopped." : result.Error);
                        AppendLog(cableStatus.Text);
                        bool held = VehicleLink.Current.IsConnected;
                        SetCableButtons(held);
                    });
                });
            }
        }

        void DisconnectClicked(object sender, EventArgs e)
        {
            transferring = false;
            VehicleLink.Current.Disconnect();
            transferProgress.Value = 0;
            SetCableButtons(false);
            cableStatus.BackColor = Color.FromArgb(223, 227, 222);
            cableStatus.Text = "Disconnected. The channel is closed.";
            AppendLog(cableStatus.Text);
        }

        void AskClicked(object sender, EventArgs e)
        {
            string question = questionBox.Text.Trim();
            if (question.Length == 0) return;
            if (question.Length > 1000) question = question.Substring(0, 1000);
            questionBox.Text = "";
            threadBox.AppendText("You: " + question + Environment.NewLine + Environment.NewLine);
            askButton.Enabled = false;
            askButton.Text = "Asking";
            Dictionary<string, object> deviceSnapshot = device;
            ThreadPool.QueueUserWorkItem(delegate
            {
                string answer = Assistant.Answer(question, definitions, deviceSnapshot);
                BeginInvoke((Action)delegate
                {
                    threadBox.AppendText("Wideband: " + answer + Environment.NewLine + Environment.NewLine);
                    askButton.Enabled = true;
                    askButton.Text = "Ask";
                });
            });
        }

        static string Value(Dictionary<string, object> source, string key)
        {
            object value;
            if (source != null && source.TryGetValue(key, out value) && value != null) return value.ToString();
            return "";
        }

        protected override void OnFormClosed(FormClosedEventArgs e)
        {
            try { VehicleLink.Current.Disconnect(); }
            catch (Exception) { }
            ui.Dispose();
            titleFont.Dispose();
            base.OnFormClosed(e);
        }
    }

    static class Program
    {
        [STAThread]
        static void Main()
        {
            Application.SetUnhandledExceptionMode(UnhandledExceptionMode.CatchException);
            Application.ThreadException += delegate(object sender, ThreadExceptionEventArgs args)
            {
                File.WriteAllText(Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "startup-error.txt"), args.Exception.ToString());
            };
            try
            {
                string install = @"C:\Program Files (x86)\VF Tuner";
                if (!File.Exists(Path.Combine(install, "Logger.ini")))
                    install = @"C:\Users\ryanp\VF-Tuner-backup";
                List<Definition> definitions = Catalog.LoadLogger(Path.Combine(install, "Logger.ini"));
                List<string> profiles = Catalog.LoadProfiles(Path.Combine(install, "ToyotaProfiles"));
                Application.EnableVisualStyles();
                Application.SetCompatibleTextRenderingDefault(false);
                Application.Run(new MainForm(install, definitions, profiles));
            }
            catch (Exception ex)
            {
                File.WriteAllText(Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "startup-error.txt"), ex.ToString());
            }
        }
    }
}
