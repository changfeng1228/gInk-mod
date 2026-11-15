using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Threading;
using System.Windows.Forms;

namespace gInk
{
	// WM_TIMER can remain coalesced to about 15.6 ms even with timeBeginPeriod(1).
	// This scheduler uses high resolution only for short active intervals and never
	// posts another callback while the previous UI callback is still running.
	public sealed class HighResolutionTimer : Component
	{
		private readonly object stateLock = new object();
		private readonly Control target;
		private volatile bool enabled;
		private volatile int generation;
		private volatile int interval = 15;
		private bool disposed;
		private int callbackPending;
		private bool highResolutionActive;
		private bool resolutionAcquired;

		public HighResolutionTimer()
		{
		}

		public HighResolutionTimer(IContainer container, Control target)
		{
			this.target = target;
			container.Add(this);
			target.HandleDestroyed += delegate
			{
				Interlocked.Exchange(ref callbackPending, 0);
			};
		}

		public int Interval
		{
			get { return interval; }
			set { interval = Math.Max(1, value); }
		}

		public bool Enabled
		{
			get { return enabled; }
			set
			{
				if (value)
					Start();
				else
					Stop();
			}
		}

		public event EventHandler Tick;

		public void Start()
		{
			lock (stateLock)
			{
				if (disposed)
					throw new ObjectDisposedException(GetType().FullName);
				if (enabled)
					return;

				int currentGeneration = ++generation;
				enabled = true;
				Thread worker = new Thread(Run);
				worker.IsBackground = true;
				worker.Name = "gInk high-resolution UI timer";
				worker.Start(currentGeneration);
			}
		}

		public void Stop()
		{
			lock (stateLock)
			{
				if (!enabled)
					return;
				generation++;
				enabled = false;
			}
		}

		private void Run(object state)
		{
			int currentGeneration = (int)state;
			try
			{
				while (IsRunning(currentGeneration))
				{
					UpdateTimerResolution(Interval <= 5);
					Thread.Sleep(Math.Max(1, Interval - 1));
					if (IsRunning(currentGeneration))
						Dispatch(currentGeneration);
				}
			}
			finally
			{
				ReleaseTimerResolution();
			}
		}

		private bool IsRunning(int currentGeneration)
		{
			return enabled && generation == currentGeneration;
		}

		private void Dispatch(int currentGeneration)
		{
			if (Interlocked.CompareExchange(ref callbackPending, 1, 0) != 0)
				return;
			if (target == null || !target.IsHandleCreated)
			{
				Interlocked.Exchange(ref callbackPending, 0);
				return;
			}

			try
			{
				target.BeginInvoke((MethodInvoker)delegate
				{
					try
					{
						if (!IsRunning(currentGeneration) || target.IsDisposed || target.Disposing)
							return;
						EventHandler handler = Tick;
						if (handler != null)
							handler(this, EventArgs.Empty);
					}
					finally
					{
						Interlocked.Exchange(ref callbackPending, 0);
					}
				});
			}
			catch (InvalidOperationException)
			{
				Interlocked.Exchange(ref callbackPending, 0);
			}
		}

		protected override void Dispose(bool disposing)
		{
			if (disposing)
			{
				lock (stateLock)
				{
					disposed = true;
					generation++;
					enabled = false;
				}
			}
			base.Dispose(disposing);
		}

		private void UpdateTimerResolution(bool required)
		{
			if (required == highResolutionActive)
				return;
			if (required)
				resolutionAcquired = timeBeginPeriod(1) == 0;
			else if (resolutionAcquired)
			{
				timeEndPeriod(1);
				resolutionAcquired = false;
			}
			highResolutionActive = required;
		}

		private void ReleaseTimerResolution()
		{
			if (highResolutionActive && resolutionAcquired)
				timeEndPeriod(1);
			highResolutionActive = false;
			resolutionAcquired = false;
		}

		[DllImport("winmm.dll")]
		private static extern uint timeBeginPeriod(uint period);
		[DllImport("winmm.dll")]
		private static extern uint timeEndPeriod(uint period);
	}
}
