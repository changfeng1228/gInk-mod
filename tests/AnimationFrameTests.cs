using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Windows.Forms;

internal static class AnimationFrameTests
{
    [STAThread]
    private static int Main(string[] args)
    {
        Application.SetUnhandledExceptionMode(UnhandledExceptionMode.ThrowException);
        if (args.Length != 1)
        {
            Console.Error.WriteLine("Usage: AnimationFrameTests.exe <gInk.exe>");
            return 2;
        }

        string appDirectory = Path.GetDirectoryName(Path.GetFullPath(args[0]));
        Directory.SetCurrentDirectory(appDirectory);
        AppDomain.CurrentDomain.AssemblyResolve += delegate(object sender, ResolveEventArgs resolveArgs)
        {
            string dependency = Path.Combine(appDirectory, new AssemblyName(resolveArgs.Name).Name + ".dll");
            return File.Exists(dependency) ? Assembly.LoadFrom(dependency) : null;
        };

        Assembly assembly = Assembly.LoadFrom(args[0]);
        Type rootType = assembly.GetType("gInk.Root", true);
        object root = Activator.CreateInstance(rootType);
        var samples = new Dictionary<string, List<double>>();
        var entranceSteps = new List<int>();
        var watch = Stopwatch.StartNew();
        bool timersStopped = false;
        bool idleCadenceRestored = false;
        int cooldownAfter300 = 0;
        int cooldownAfter1200 = 0;

        try
        {
            rootType.GetMethod("StartInk").Invoke(root, null);
            object collection = rootType.GetField("FormCollection").GetValue(root);
            object display = rootType.GetField("FormDisplay").GetValue(root);
            var timers = new[]
            {
                new { Name = "toolbar logic", Timer = GetTimer(collection, "tiSlide") },
                new { Name = "display callback", Timer = GetTimer(display, "timer1") }
            };
            foreach (var item in timers)
                Console.WriteLine("{0} requested interval={1} ms", item.Name, item.Timer.Interval);

            bool entranceCompleted = false;
            foreach (var item in timers)
            {
                var intervals = new List<double>();
                samples[item.Name] = intervals;
                double previous = watch.Elapsed.TotalMilliseconds;
                item.Timer.Tick += delegate
                {
                    double now = watch.Elapsed.TotalMilliseconds;
                    if (!entranceCompleted)
                        intervals.Add(now - previous);
                    previous = now;
                };
            }

            Type collectionType = collection.GetType();
            var buttons = (Panel)collectionType.GetField("gpButtons").GetValue(collection);
            var enteringField = collectionType.GetField("ButtonsEntering");
            int previousLeft = buttons.Left;
            timers[0].Timer.Tick += delegate
            {
                if (entranceCompleted)
                    return;

                int step = Math.Abs(buttons.Left - previousLeft);
                if (step > 0)
                    entranceSteps.Add(step);
                previousLeft = buttons.Left;
                if ((int)enteringField.GetValue(collection) == 0 && entranceSteps.Count > 0)
                    entranceCompleted = true;
            };

            watch.Restart();
            while (watch.ElapsedMilliseconds < 1500)
                Application.DoEvents();
            idleCadenceRestored = timers.All(item => item.Timer.Interval >= 15);

            collectionType.GetMethod("ExitSnapping").Invoke(collection, null);
            var cooldownWatch = Stopwatch.StartNew();
            while (cooldownWatch.ElapsedMilliseconds < 300)
                Application.DoEvents();
            cooldownAfter300 = (int)rootType.GetField("Snapping").GetValue(root);
            while (cooldownWatch.ElapsedMilliseconds < 1200)
                Application.DoEvents();
            cooldownAfter1200 = (int)rootType.GetField("Snapping").GetValue(root);

            collectionType.GetMethod("RetreatAndExit").Invoke(collection, null);
            var exitWatch = Stopwatch.StartNew();
            while (rootType.GetField("FormCollection").GetValue(root) != null && exitWatch.ElapsedMilliseconds < 2500)
                Application.DoEvents();
            if (rootType.GetField("FormCollection").GetValue(root) != null)
                throw new Exception("Exit animation did not close the ink forms within 2.5 seconds.");
            timersStopped = timers.All(item => !item.Timer.Enabled);
        }
        finally
        {
            try
            {
                if (rootType.GetField("FormCollection").GetValue(root) != null)
                    rootType.GetMethod("StopInk").Invoke(root, null);
            }
            finally
            {
                var trayIcon = (NotifyIcon)rootType.GetField("trayIcon").GetValue(root);
                trayIcon.Visible = false;
                trayIcon.Dispose();
            }
        }

        var failed = false;
        foreach (var pair in samples)
        {
            double[] ordered = pair.Value.Skip(3).OrderBy(value => value).ToArray();
            if (ordered.Length < 20)
                throw new Exception(pair.Key + " produced only " + ordered.Length + " timer samples.");

            double median = ordered[ordered.Length / 2];
            double p95 = ordered[(int)(ordered.Length * 0.95)];
            bool passed = median <= 6.0 && p95 <= 12.0;
            Console.WriteLine("{0}: median {1:F2} ms ({2:F1} callbacks/s), p95 {3:F2} ms - {4}",
                pair.Key, median, 1000.0 / median, p95, passed ? "PASS" : "FAIL");
            failed |= !passed;
        }

        if (entranceSteps.Count < 5)
            throw new Exception("Entrance produced only " + entranceSteps.Count + " movement samples.");
        int largestEarlyStep = entranceSteps.Take(8).Max();
        bool smoothEntrance = largestEarlyStep <= 30;
        Console.WriteLine("entrance motion: largest of first 8 steps {0} px - {1}",
            largestEarlyStep, smoothEntrance ? "PASS" : "FAIL");
        failed |= !smoothEntrance;

        bool cooldownPreserved = cooldownAfter300 >= -50 && cooldownAfter300 <= -30 && cooldownAfter1200 == 0;
        Console.WriteLine("snapshot cooldown: {0} after 300 ms, {1} after 1200 ms - {2}",
            cooldownAfter300, cooldownAfter1200, cooldownPreserved ? "PASS" : "FAIL");
        failed |= !cooldownPreserved;

        Console.WriteLine("idle cadence restored to 15 ms: {0}", idleCadenceRestored ? "PASS" : "FAIL");
        failed |= !idleCadenceRestored;

        Console.WriteLine("exit animation and timer shutdown: {0}", timersStopped ? "PASS" : "FAIL");
        failed |= !timersStopped;

        return failed ? 1 : 0;
    }

    private static TimerProbe GetTimer(object form, string fieldName)
    {
        object timer = form.GetType().GetField(fieldName,
            BindingFlags.Instance | BindingFlags.NonPublic | BindingFlags.Public).GetValue(form);
        return new TimerProbe(timer);
    }

    private sealed class TimerProbe
    {
        private readonly object instance;
        private readonly EventInfo tickEvent;

        internal TimerProbe(object instance)
        {
            this.instance = instance;
            tickEvent = instance.GetType().GetEvent("Tick");
            if (tickEvent == null)
                throw new Exception(instance.GetType().FullName + " has no Tick event.");
        }

        internal double Interval
        {
            get { return Convert.ToDouble(instance.GetType().GetProperty("Interval").GetValue(instance, null)); }
        }

        internal bool Enabled
        {
            get { return (bool)instance.GetType().GetProperty("Enabled").GetValue(instance, null); }
        }

        internal event EventHandler Tick
        {
            add { tickEvent.AddEventHandler(instance, value); }
            remove { tickEvent.RemoveEventHandler(instance, value); }
        }
    }
}
