using System;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;
using System.Windows.Forms;
using Microsoft.Win32;

[assembly: System.Reflection.AssemblyTitle("NestDesktop Setup")]
[assembly: System.Reflection.AssemblyProduct("NestDesktop")]
[assembly: System.Reflection.AssemblyVersion("0.2.11.0")]
[assembly: System.Reflection.AssemblyFileVersion("0.2.11.0")]

internal static class Program
{
    [STAThread]
    private static void Main(string[] args)
    {
        Application.EnableVisualStyles();
        Application.SetCompatibleTextRenderingDefault(false);
        try
        {
            bool uninstall = args.Any(a => a.Equals("--uninstall", StringComparison.OrdinalIgnoreCase));
            if (uninstall && Installer.IsInsideInstallDirectory(Application.ExecutablePath))
            {
                // A temporary copy can remove the installed setup executable after
                // this process exits. No shell command or scheduled task is needed.
                string temporary = Path.Combine(Path.GetTempPath(), "NestDesktop-uninstall-" + Guid.NewGuid().ToString("N") + ".exe");
                File.Copy(Application.ExecutablePath, temporary, false);
                Process.Start(new ProcessStartInfo(temporary, "--uninstall") { UseShellExecute = false });
                return;
            }
            Application.Run(new SetupWindow(uninstall));
        }
        catch (Exception error)
        {
            MessageBox.Show(error.Message, "NestDesktop Setup", MessageBoxButtons.OK, MessageBoxIcon.Error);
        }
    }
}

internal sealed class SetupWindow : Form
{
    private const int SW_RESTORE = 9;
    private const uint SWP_NOSIZE = 0x0001;
    private const uint SWP_NOMOVE = 0x0002;
    private const uint SWP_SHOWWINDOW = 0x0040;

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool ShowWindow(IntPtr window, int command);

    [DllImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool SetWindowPos(IntPtr window, IntPtr insertAfter, int x, int y, int width, int height, uint flags);

    private readonly bool uninstall;
    private readonly CheckBox startup;
    private readonly CheckBox launch;
    private readonly Button action;
    private readonly Label status;
    private bool finished;

    public SetupWindow(bool remove)
    {
        SuspendLayout();
        uninstall = remove;
        Text = remove ? "Uninstall NestDesktop" : "Install NestDesktop";
        Icon = Icon.ExtractAssociatedIcon(Application.ExecutablePath);
        FormBorderStyle = FormBorderStyle.FixedDialog;
        MaximizeBox = false;
        MinimizeBox = false;
        StartPosition = FormStartPosition.CenterScreen;
        // All dimensions below are authored at 96 DPI. Let rows grow to their
        // measured text height instead of clipping labels in fixed rectangles.
        AutoScaleDimensions = new SizeF(96F, 96F);
        AutoScaleMode = AutoScaleMode.Dpi;
        AutoSize = true;
        AutoSizeMode = AutoSizeMode.GrowAndShrink;
        AutoScroll = true;
        BackColor = Color.FromArgb(246, 247, 251);
        Font = new Font("Segoe UI", 10F);

        TableLayoutPanel layout = Column();
        layout.Dock = DockStyle.None;
        layout.Padding = new Padding(30, 29, 30, 24);
        layout.MinimumSize = new Size(560, 0);
        layout.MaximumSize = new Size(560, 0);
        Controls.Add(layout);

        TableLayoutPanel header = new TableLayoutPanel {
            AutoSize = true, AutoSizeMode = AutoSizeMode.GrowAndShrink,
            Dock = DockStyle.Fill, ColumnCount = 2, RowCount = 1,
            Margin = new Padding(0, 0, 0, 28)
        };
        header.ColumnStyles.Add(new ColumnStyle(SizeType.AutoSize));
        header.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100F));
        PictureBox mark = new PictureBox { Size = new Size(58, 58), Margin = new Padding(0, 0, 16, 0) };
        mark.Paint += DrawMark;
        header.Controls.Add(mark, 0, 0);
        TableLayoutPanel heading = Column();
        heading.Margin = Padding.Empty;
        Label title = Paragraph("NestDesktop", Color.FromArgb(30, 34, 52), 426);
        title.Font = new Font("Segoe UI", 23F, FontStyle.Bold);
        heading.Controls.Add(title);
        Label subtitle = Paragraph("App folders on your desktop", Color.FromArgb(97, 104, 124), 426);
        subtitle.Margin = new Padding(0, 3, 0, 0);
        heading.Controls.Add(subtitle);
        header.Controls.Add(heading, 1, 0);
        layout.Controls.Add(header);

        TableLayoutPanel card = Column();
        card.BackColor = Color.White;
        card.Padding = new Padding(20);
        card.Margin = Padding.Empty;
        Label cardTitle = Paragraph(remove ? "Uninstall NestDesktop" : "Install desktop folders", Color.FromArgb(35, 39, 56), 460);
        cardTitle.Font = new Font("Segoe UI", 13F, FontStyle.Bold);
        cardTitle.Margin = new Padding(0, 0, 0, 14);
        card.Controls.Add(cardTitle);
        card.Controls.Add(Paragraph(
            remove ? "The application and its desktop menu will be removed.\nCollected shortcuts are restored without overwriting existing files.\nSaved folders and layouts are kept."
                : "Small app-folder tiles on your normal Windows desktop.\nRight-click the desktop to create a folder.",
            Color.FromArgb(92, 99, 118), 460));
        startup = new CheckBox { Text = "Start NestDesktop when I sign in", Checked = true, AutoSize = true, MaximumSize = new Size(460, 0), Dock = DockStyle.Fill, Margin = new Padding(0, 20, 0, 0), ForeColor = Color.FromArgb(46, 51, 69) };
        launch = new CheckBox { Text = "Show desktop folders after installation", Checked = true, AutoSize = true, MaximumSize = new Size(460, 0), Dock = DockStyle.Fill, Margin = new Padding(0, 12, 0, 0), ForeColor = Color.FromArgb(46, 51, 69) };
        if (remove)
        {
            Label confirmation = Paragraph("Select Uninstall to close and remove NestDesktop.", Color.FromArgb(92, 99, 118), 460);
            confirmation.Margin = new Padding(0, 20, 0, 0);
            card.Controls.Add(confirmation);
        }
        else
        {
            card.Controls.Add(startup);
            card.Controls.Add(launch);
        }
        layout.Controls.Add(card);

        TableLayoutPanel footer = new TableLayoutPanel {
            AutoSize = true, AutoSizeMode = AutoSizeMode.GrowAndShrink,
            Dock = DockStyle.Fill, ColumnCount = 2, RowCount = 1,
            Margin = new Padding(0, 22, 0, 0)
        };
        footer.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100F));
        footer.ColumnStyles.Add(new ColumnStyle(SizeType.AutoSize));
        status = Paragraph(remove ? "Existing desktop files are never overwritten." : "Install location: Local AppData / Programs / NestDesktop", Color.FromArgb(105, 111, 131), 336);
        status.Font = new Font("Segoe UI", 8.5F);
        status.Margin = new Padding(0, 0, 24, 0);
        status.Anchor = AnchorStyles.Left;
        footer.Controls.Add(status, 0, 0);
        action = new Button { Text = remove ? "Uninstall" : "Install", AutoSize = true, MinimumSize = new Size(140, 43), Margin = Padding.Empty, Padding = new Padding(16, 8, 16, 8), Anchor = AnchorStyles.Top | AnchorStyles.Right, BackColor = remove ? Color.FromArgb(181, 62, 77) : Color.FromArgb(109, 91, 211), ForeColor = Color.White, FlatStyle = FlatStyle.Flat, Font = new Font("Segoe UI", 10.5F, FontStyle.Bold), Cursor = Cursors.Hand };
        action.FlatAppearance.BorderSize = 0;
        if (remove)
        {
            action.FlatAppearance.MouseOverBackColor = Color.FromArgb(159, 49, 64);
            action.FlatAppearance.MouseDownBackColor = Color.FromArgb(143, 42, 57);
        }
        action.Click += RunAction;
        footer.Controls.Add(action, 1, 0);
        layout.Controls.Add(footer);
        AcceptButton = action;
        ResumeLayout(true);
    }

    private static TableLayoutPanel Column()
    {
        TableLayoutPanel column = new TableLayoutPanel {
            AutoSize = true, AutoSizeMode = AutoSizeMode.GrowAndShrink,
            Dock = DockStyle.Top, ColumnCount = 1, RowCount = 0,
            GrowStyle = TableLayoutPanelGrowStyle.AddRows, Margin = Padding.Empty
        };
        column.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100F));
        return column;
    }

    private static Label Paragraph(string text, Color color, int width)
    {
        return new Label {
            Text = text, ForeColor = color, AutoSize = true,
            MaximumSize = new Size(width, 0), Dock = DockStyle.Fill,
            Margin = Padding.Empty, UseMnemonic = false
        };
    }

    protected override void OnShown(EventArgs e)
    {
        base.OnShown(e);
        // Large display/text scaling can make even correctly wrapped content
        // taller than a laptop's work area. Keep the native window on screen;
        // its scrollable content still exposes the full text and action button.
        Rectangle workArea = Screen.FromControl(this).WorkingArea;
        MaximumSize = workArea.Size;
        PerformLayout();
        Location = new Point(
            Math.Max(workArea.Left, Math.Min(Left, workArea.Right - Width)),
            Math.Max(workArea.Top, Math.Min(Top, workArea.Bottom - Height)));
        // A background launcher can supply a hidden initial show state. After
        // WinForms has consumed that first show, display this confirmation once
        // without moving it or changing its size.
        ShowWindow(Handle, SW_RESTORE);
        SetWindowPos(Handle, IntPtr.Zero, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW);
        Activate();
    }

    private void DrawMark(object sender, PaintEventArgs e)
    {
        Control mark = (Control)sender;
        e.Graphics.ScaleTransform(mark.ClientSize.Width / 58F, mark.ClientSize.Height / 58F);
        e.Graphics.SmoothingMode = SmoothingMode.AntiAlias;
        using (SolidBrush back = new SolidBrush(Color.FromArgb(201, 188, 255)))
        using (GraphicsPath shape = Rounded(new RectangleF(1, 1, 55, 55), 14))
            e.Graphics.FillPath(back, shape);
        Color[] colors = { Color.FromArgb(108, 90, 210), Color.FromArgb(143, 125, 231), Color.FromArgb(97, 159, 178), Color.FromArgb(233, 173, 117) };
        for (int i = 0; i < 4; i++)
        using (SolidBrush brush = new SolidBrush(colors[i]))
        using (GraphicsPath dot = Rounded(new RectangleF(12 + (i % 2) * 18, 12 + (i / 2) * 18, 14, 14), 4))
            e.Graphics.FillPath(brush, dot);
    }

    private static GraphicsPath Rounded(RectangleF rect, float radius)
    {
        GraphicsPath path = new GraphicsPath();
        float diameter = radius * 2;
        path.AddArc(rect.Left, rect.Top, diameter, diameter, 180, 90);
        path.AddArc(rect.Right - diameter, rect.Top, diameter, diameter, 270, 90);
        path.AddArc(rect.Right - diameter, rect.Bottom - diameter, diameter, diameter, 0, 90);
        path.AddArc(rect.Left, rect.Bottom - diameter, diameter, diameter, 90, 90);
        path.CloseFigure();
        return path;
    }

    private void RunAction(object sender, EventArgs args)
    {
        if (finished) { Close(); return; }
        action.Enabled = false;
        status.Text = uninstall ? "Closing NestDesktop and removing application..." : "Installing application...";
        status.ForeColor = Color.FromArgb(105, 111, 131);
        Application.DoEvents();
        try
        {
            if (uninstall)
                Installer.Remove();
            else
                Installer.Install(Application.StartupPath, startup.Checked);
            finished = true;
            startup.Enabled = false;
            launch.Enabled = false;
            action.Text = "Done";
            status.Text = uninstall ? "Removed. Collected shortcuts restored where possible; saved folders kept."
                : launch.Checked ? "Installed. Your folders are on the desktop."
                : "Installed. Open NestDesktop to show desktop folders.";
            if (!uninstall && launch.Checked)
                Process.Start(new ProcessStartInfo(Installer.ExecutablePath, "--reveal") { UseShellExecute = true });
        }
        catch (Exception error)
        {
            status.Text = error.Message;
            status.ForeColor = Color.FromArgb(165, 49, 61);
        }
        finally { action.Enabled = true; }
    }
}

internal static class Installer
{
    private const string RunKey = @"Software\Microsoft\Windows\CurrentVersion\Run";
    private const string UninstallKey = @"Software\Microsoft\Windows\CurrentVersion\Uninstall\NestDesktop";
    private const string DesktopMenuKey = @"Software\Classes\DesktopBackground\Shell\NestDesktop";
    internal static readonly string InstallDirectory = Path.GetFullPath(Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "Programs", "NestDesktop"));
    internal static readonly string ExecutablePath = Path.Combine(InstallDirectory, "nest-desktop.exe");
    private static readonly string SetupPath = Path.Combine(InstallDirectory, "NestDesktop-Setup.exe");
    private static readonly string ShortcutPath = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.Programs), "NestDesktop.lnk");

    internal static bool IsInsideInstallDirectory(string path)
    {
        return Path.GetDirectoryName(Path.GetFullPath(path)).Equals(InstallDirectory, StringComparison.OrdinalIgnoreCase);
    }

    private static void CheckRunning()
    {
        foreach (Process process in Process.GetProcessesByName("nest-desktop"))
        {
            using (process)
            {
                string location = null;
                try { location = process.MainModule.FileName; } catch { }
                if (location != null && Path.GetFullPath(location).Equals(ExecutablePath, StringComparison.OrdinalIgnoreCase))
                    throw new InvalidOperationException("Quit NestDesktop from its tray menu, then try again.");
            }
        }
    }

    private static void ValidateTarget()
    {
        string programsRoot = Path.GetFullPath(Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "Programs"));
        if (!Path.GetDirectoryName(InstallDirectory).Equals(programsRoot, StringComparison.OrdinalIgnoreCase) || !Path.GetFileName(InstallDirectory).Equals("NestDesktop", StringComparison.Ordinal))
            throw new InvalidOperationException("The installation location could not be verified.");
        if (Directory.Exists(InstallDirectory) && (File.GetAttributes(InstallDirectory) & FileAttributes.ReparsePoint) != 0)
            throw new InvalidOperationException("The installation location is a link. Choose a normal local folder.");
    }

    private static void CopySafely(string source, string destination)
    {
        if (Path.GetFullPath(source).Equals(Path.GetFullPath(destination), StringComparison.OrdinalIgnoreCase)) return;
        string staging = destination + ".installing";
        File.Copy(source, staging, true);
        if (File.Exists(destination)) File.Replace(staging, destination, null);
        else File.Move(staging, destination);
    }

    internal static void Install(string sourceDirectory, bool startup)
    {
        ValidateTarget();
        CheckRunning();
        string sourceExecutable = Path.Combine(sourceDirectory, "nest-desktop.exe");
        if (!File.Exists(sourceExecutable))
            throw new FileNotFoundException("Extract the whole release ZIP first. nest-desktop.exe must be next to this setup program.");
        Directory.CreateDirectory(InstallDirectory);
        CopySafely(sourceExecutable, ExecutablePath);
        CopySafely(Application.ExecutablePath, SetupPath);
        foreach (string name in new[] { "README.md", "LICENSE", "THIRD_PARTY_NOTICES.md" })
        {
            string source = Path.Combine(sourceDirectory, name);
            if (File.Exists(source)) CopySafely(source, Path.Combine(InstallDirectory, name));
        }
        CreateShortcut();
        using (RegistryKey run = Registry.CurrentUser.CreateSubKey(RunKey))
        {
            if (startup) run.SetValue("NestDesktop", "\"" + ExecutablePath + "\" --background", RegistryValueKind.String);
            else run.DeleteValue("NestDesktop", false);
        }
        using (RegistryKey desktopMenu = Registry.CurrentUser.CreateSubKey(DesktopMenuKey))
        {
            desktopMenu.SetValue("MUIVerb", "New app folder", RegistryValueKind.String);
            desktopMenu.SetValue("Icon", ExecutablePath, RegistryValueKind.String);
            using (RegistryKey command = desktopMenu.CreateSubKey("command"))
                command.SetValue("", "\"" + ExecutablePath + "\" --new-folder", RegistryValueKind.String);
        }
        using (RegistryKey entry = Registry.CurrentUser.CreateSubKey(UninstallKey))
        {
            entry.SetValue("DisplayName", "NestDesktop");
            entry.SetValue("DisplayVersion", "0.2.11");
            entry.SetValue("Publisher", "NestDesktop");
            entry.SetValue("InstallLocation", InstallDirectory);
            entry.SetValue("DisplayIcon", ExecutablePath);
            entry.SetValue("UninstallString", "\"" + SetupPath + "\" --uninstall");
            entry.SetValue("NoModify", 1, RegistryValueKind.DWord);
            entry.SetValue("NoRepair", 1, RegistryValueKind.DWord);
            entry.SetValue("EstimatedSize", (int)Math.Max(1, new FileInfo(ExecutablePath).Length / 1024), RegistryValueKind.DWord);
        }
    }

    private static void CreateShortcut()
    {
        Directory.CreateDirectory(Path.GetDirectoryName(ShortcutPath));
        object shellObject = Activator.CreateInstance(Type.GetTypeFromProgID("WScript.Shell"));
        object shortcutObject = null;
        try
        {
            dynamic shell = shellObject;
            shortcutObject = shell.CreateShortcut(ShortcutPath);
            dynamic shortcut = shortcutObject;
            shortcut.TargetPath = ExecutablePath;
            shortcut.WorkingDirectory = InstallDirectory;
            shortcut.Description = "Android-inspired folders for your Windows desktop";
            shortcut.IconLocation = ExecutablePath + ",0";
            shortcut.Save();
        }
        finally
        {
            if (shortcutObject != null) Marshal.FinalReleaseComObject(shortcutObject);
            Marshal.FinalReleaseComObject(shellObject);
        }
    }

    internal static void Remove()
    {
        ValidateTarget();
        StopForRemoval();
        CheckRunning();
        RestoreCollectedShortcuts();
        // Remove only files installed by this program. A user's extra files,
        // including anything they placed in this directory, are never recursed.
        foreach (string name in new[] { "nest-desktop.exe", "NestDesktop-Setup.exe", "README.md", "LICENSE", "THIRD_PARTY_NOTICES.md", "nest-desktop.exe.installing", "NestDesktop-Setup.exe.installing" })
        {
            string path = Path.Combine(InstallDirectory, name);
            if (File.Exists(path)) File.Delete(path);
        }
        if (Directory.Exists(InstallDirectory) && !Directory.EnumerateFileSystemEntries(InstallDirectory).Any())
            Directory.Delete(InstallDirectory, false);
        using (RegistryKey run = Registry.CurrentUser.OpenSubKey(RunKey, true))
            if (run != null) run.DeleteValue("NestDesktop", false);
        Registry.CurrentUser.DeleteSubKeyTree(UninstallKey, false);
        Registry.CurrentUser.DeleteSubKeyTree(DesktopMenuKey, false);
        if (File.Exists(ShortcutPath)) File.Delete(ShortcutPath);
    }

    private static void StopForRemoval()
    {
        const int timeoutMilliseconds = 10000;
        Stopwatch elapsed = Stopwatch.StartNew();
        if (File.Exists(ExecutablePath))
        {
            // Ask the installed application to save and quit. Its watchdog must
            // also finish before files can be restored or removed; never kill
            // either process and bypass shortcut recovery.
            using (Process request = Process.Start(new ProcessStartInfo(ExecutablePath, "--quit --background") {
                UseShellExecute = false,
                CreateNoWindow = true,
                WindowStyle = ProcessWindowStyle.Hidden,
                WorkingDirectory = InstallDirectory
            }))
            {
                if (request == null)
                    throw new InvalidOperationException("NestDesktop could not be asked to close. No application files have been removed.");
                int remaining = Math.Max(0, timeoutMilliseconds - (int)elapsed.ElapsedMilliseconds);
                if (!request.WaitForExit(remaining) || request.ExitCode != 0)
                    throw new InvalidOperationException("NestDesktop has not finished closing. Wait and try Uninstall again. No application files have been removed.");
            }
        }

        while (true)
        {
            Process[] running = Process.GetProcessesByName("nest-desktop");
            if (running.Length == 0) return;
            try
            {
                foreach (Process process in running)
                {
                    int remaining = Math.Max(0, timeoutMilliseconds - (int)elapsed.ElapsedMilliseconds);
                    if (!process.WaitForExit(remaining))
                        throw new InvalidOperationException("NestDesktop has not finished closing. Wait and try Uninstall again. No application files have been removed.");
                }
            }
            finally
            {
                foreach (Process process in running) process.Dispose();
            }
            // Enumerate again after exit so a watchdog starting during shutdown
            // cannot be missed by the first process snapshot.
            if (elapsed.ElapsedMilliseconds >= timeoutMilliseconds)
                throw new InvalidOperationException("NestDesktop has not finished closing. Wait and try Uninstall again. No application files have been removed.");
        }
    }

    private static void RestoreCollectedShortcuts()
    {
        if (!File.Exists(ExecutablePath)) return;
        using (Process restore = Process.Start(new ProcessStartInfo(ExecutablePath, "--restore-shortcuts --background") {
            UseShellExecute = false,
            CreateNoWindow = true,
            WindowStyle = ProcessWindowStyle.Hidden,
            WorkingDirectory = InstallDirectory
        }))
        {
            if (restore == null)
                throw new InvalidOperationException("Shortcut restore could not start. Nothing has been uninstalled.");
            if (!restore.WaitForExit(30000))
                throw new InvalidOperationException("Shortcut restore has not finished. Wait for it to finish, then retry uninstalling. Program files and layouts were kept.");
            if (restore.ExitCode != 0)
                throw new InvalidOperationException("Shortcut restore reported an error. Program files and layouts were kept. Open NestDesktop to restore the shortcuts, then retry.");
        }
    }
}

