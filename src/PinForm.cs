using System;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Drawing.Imaging;
using System.Windows.Forms;
using System.Runtime.InteropServices;

namespace gInk
{
	/// <summary>
	/// Runs a block of geometry code in a per-monitor-DPI-aware context.
	///
	/// The pin window is created while the thread is DPI aware (see Root.AddPin) and the context
	/// is restored immediately after, so every later call runs on a DPI-unaware thread. Measured
	/// on a 150% display: writing SetBounds(..., 900, 600) from that thread produces a 1350x900
	/// window, while Bounds reads back 1350x900 — writes are scaled, reads are not, so the two
	/// stop agreeing. Every read and write of geometry in this class therefore goes through this
	/// scope, which keeps all coordinates in real screen pixels and matching the size the window
	/// was created with.
	/// </summary>
	internal struct DpiScope : IDisposable
	{
		const int PER_MONITOR_AWARE_V2 = -4;
		IntPtr previous;
		bool active;

		[DllImport("user32.dll")]
		static extern IntPtr SetThreadDpiAwarenessContext(IntPtr dpiContext);

		public static DpiScope Enter()
		{
			DpiScope s = new DpiScope();
			try
			{
				s.previous = SetThreadDpiAwarenessContext(new IntPtr(PER_MONITOR_AWARE_V2));
				s.active = s.previous != IntPtr.Zero;
			}
			catch (EntryPointNotFoundException)
			{
				// Windows 7/8: no per-window DPI contexts. Nothing is scaled there either,
				// so plain coordinates are already correct.
				s.active = false;
			}
			return s;
		}

		public void Dispose()
		{
			if (!active)
				return;
			active = false;
			try { SetThreadDpiAwarenessContext(previous); }
			catch (EntryPointNotFoundException) { }
		}
	}

	/// <summary>
	/// A snapshot pinned on top of the desktop, for comparing against whatever is underneath.
	/// Left drag moves it, dragging a corner resizes it (keeping the aspect ratio), wheel zooms,
	/// Alt+wheel changes opacity, right click for the menu, double click or Esc closes it.
	/// Holding Alt lets clicks fall through to the window below.
	/// </summary>
	public class PinForm : Form
	{
		public Root Root;
		Bitmap image;
		float zoom = 1f;
		bool dragging = false;
		Point dragStart;      // screen point at mouse down
		Point formStart;      // window location at mouse down
		DateTime showtime;

		// corner resizing: 0 none, 1 top-left, 2 top-right, 3 bottom-left, 4 bottom-right
		const int CornerGrab = 12;   // size of the corner hot zone, in pixels
		const float MinZoom = 0.05f;
		const float MaxZoom = 8f;
		const int MinEdge = 24;      // never shrink below this, or the pin cannot be grabbed again
		int resizingCorner = 0;
		Rectangle resizeStartBounds;
		float resizeStartZoom;
		int hoverCorner = 0;

		const int WM_NCHITTEST = 0x84;
		const int HTTRANSPARENT = -1;

		protected override CreateParams CreateParams
		{
			get
			{
				CreateParams cp = base.CreateParams;
				cp.ExStyle |= 0x80;    // WS_EX_TOOLWINDOW: keep out of alt-tab
				cp.ExStyle |= 0x08000000; // WS_EX_NOACTIVATE: do not steal focus from the app being compared
				return cp;
			}
		}

		protected override bool ShowWithoutActivation
		{
			get { return true; }
		}

		public PinForm(Root root, Bitmap bmp, Rectangle physicalRect)
		{
			Root = root;
			image = new Bitmap(bmp); // own a copy: the caller's bitmap gets saved/disposed elsewhere

			this.FormBorderStyle = FormBorderStyle.None;
			this.ShowInTaskbar = false;
			this.TopMost = root.PinTopMost;
			this.StartPosition = FormStartPosition.Manual;
			this.DoubleBuffered = true;
			this.Text = root.Local.PinWindowTitle;

			// One image pixel per physical screen pixel: the window is created in a DPI aware
			// context here, so Windows does not stretch it on 125%/150% displays. Doing it in the
			// constructor (rather than relying on the caller) matters because a window fixes its
			// DPI behaviour when its handle is created, and the whole lifetime of the pin then has
			// to agree with that. See DpiScope for what goes wrong otherwise.
			using (DpiScope dpi = DpiScope.Enter())
			{
				this.ClientSize = new Size(Math.Max(1, image.Width), Math.Max(1, image.Height));
				this.Location = new Point(physicalRect.X, physicalRect.Y);
				// force the handle now: if it were created later, outside this scope, the window
				// would come up DPI-unaware and every coordinate below would be scaled by the
				// display factor (measured: 1200x800 became 1800x1200 at 150%).
				IntPtr h = this.Handle;
			}

			BuildMenu();
			showtime = DateTime.Now;
		}

		float Zoom
		{
			get { return zoom; }
		}

		public void SetZoom(float z)
		{
			zoom = ClampZoom(z);
			Size size = SizeForZoom(zoom);
			ApplyBounds(new Rectangle(Left, Top, size.Width, size.Height));
		}

		// Reads and writes the window rectangle in real screen pixels (see DpiScope).
		Rectangle PhysicalBounds()
		{
			using (DpiScope dpi = DpiScope.Enter())
				return Bounds;
		}

		Point PhysicalCursor()
		{
			using (DpiScope dpi = DpiScope.Enter())
				return Cursor.Position;
		}

		void ApplyBounds(Rectangle r)
		{
			using (DpiScope dpi = DpiScope.Enter())
				SetBounds(r.X, r.Y, r.Width, r.Height);
			Invalidate();
		}

		float ClampZoom(float z)
		{
			// A zoom small enough to take either side under MinEdge would leave a pin too small
			// to grab back, so the floor follows the shorter side of the image.
			int shortSide = Math.Max(1, Math.Min(image.Width, image.Height));
			float minZoom = Math.Max(MinZoom, (float)MinEdge / shortSide);
			return Math.Max(minZoom, Math.Min(MaxZoom, z));
		}

		Size SizeForZoom(float z)
		{
			return new Size(
				Math.Max(1, (int)Math.Round(image.Width * z)),
				Math.Max(1, (int)Math.Round(image.Height * z)));
		}

		ContextMenu menu;
		void BuildMenu()
		{
			menu = new ContextMenu();
			menu.MenuItems.Add(Root.Local.PinMenuCopy, delegate { CopyToClipboard(); });
			menu.MenuItems.Add(Root.Local.PinMenuSave, delegate { SaveAs(); });
			menu.MenuItems.Add("-");
			menu.MenuItems.Add(Root.Local.PinMenuZoomIn, delegate { SetZoom(zoom * 1.25f); });
			menu.MenuItems.Add(Root.Local.PinMenuZoomOut, delegate { SetZoom(zoom / 1.25f); });
			menu.MenuItems.Add(Root.Local.PinMenuResetZoom, delegate { SetZoom(1f); });
			menu.MenuItems.Add(Root.Local.PinMenuOpacity, delegate { SetOpacity(Opacity >= 1f ? 0.75f : 1f); });
			menu.MenuItems.Add(Root.Local.PinMenuTopMost, delegate { ToggleTopMost(); });
			menu.MenuItems.Add("-");
			menu.MenuItems.Add(Root.Local.PinMenuClose, delegate { ClosePin(); });
			menu.MenuItems.Add(Root.Local.PinMenuCloseAll, delegate { Root.CloseAllPins(); });
			this.ContextMenu = menu;
		}

		void ToggleTopMost()
		{
			Root.PinTopMost = !Root.PinTopMost;
			foreach (PinForm p in Root.Pins)
				p.TopMost = Root.PinTopMost;
		}

		void SetOpacity(float a)
		{
			Opacity = Math.Max(0.2f, Math.Min(1f, a));
		}

		void CopyToClipboard()
		{
			try
			{
				Clipboard.SetDataObject(image, true, 10, 100);
			}
			catch
			{
				MessageBox.Show(Root.Local.PinClipboardBusy, Root.Local.PinWindowTitle);
			}
		}

		void SaveAs()
		{
			using (SaveFileDialog dlg = new SaveFileDialog())
			{
				dlg.Filter = "PNG (*.png)|*.png";
				dlg.FileName = "pin " + DateTime.Now.ToString("yyyy-MM-dd HH-mm-ss") + ".png";
				if (dlg.ShowDialog() == DialogResult.OK)
				{
					try
					{
						image.Save(dlg.FileName, ImageFormat.Png);
					}
					catch (Exception ex)
					{
						MessageBox.Show(ex.Message, Root.Local.PinWindowTitle);
					}
				}
			}
		}

		public void ClosePin()
		{
			Root.Pins.Remove(this);
			Close();
		}

		protected override void OnPaint(PaintEventArgs e)
		{
			e.Graphics.InterpolationMode = zoom >= 1f ? InterpolationMode.NearestNeighbor : InterpolationMode.HighQualityBicubic;
			e.Graphics.PixelOffsetMode = PixelOffsetMode.HighQuality;
			e.Graphics.DrawImage(image, new Rectangle(0, 0, ClientSize.Width, ClientSize.Height));

			// Only while the pointer is on the pin: show that the corners can be dragged.
			// The hint stays out of the way the rest of the time, so it does not disturb comparing.
			if (hoverCorner != 0 || resizingCorner != 0)
				DrawResizeHint(e.Graphics);
		}

		void DrawResizeHint(Graphics g)
		{
			int w = ClientSize.Width, h = ClientSize.Height;
			if (w < 4 || h < 4)
				return;
			using (Pen edge = new Pen(Color.FromArgb(170, 40, 130, 220), 1f))
			using (Brush box = new SolidBrush(Color.FromArgb(210, 40, 130, 220)))
			{
				g.DrawRectangle(edge, 0, 0, w - 1, h - 1);
				int n = Math.Min(6, Math.Max(2, Math.Min(w, h) / 6));
				g.FillRectangle(box, 0, 0, n, n);
				g.FillRectangle(box, w - n, 0, n, n);
				g.FillRectangle(box, 0, h - n, n, n);
				g.FillRectangle(box, w - n, h - n, n, n);
			}
		}

		int HitCorner(Point p)
		{
			int w = ClientSize.Width, h = ClientSize.Height;
			int grabX = Math.Min(CornerGrab, Math.Max(1, w / 3));
			int grabY = Math.Min(CornerGrab, Math.Max(1, h / 3));
			bool left = p.X <= grabX;
			bool right = p.X >= w - grabX;
			bool top = p.Y <= grabY;
			bool bottom = p.Y >= h - grabY;
			if (left && top) return 1;
			if (right && top) return 2;
			if (left && bottom) return 3;
			if (right && bottom) return 4;
			return 0;
		}

		Cursor CursorForCorner(int corner)
		{
			switch (corner)
			{
				case 1:
				case 4:
					return Cursors.SizeNWSE;
				case 2:
				case 3:
					return Cursors.SizeNESW;
			}
			return Cursors.Default;
		}

		protected override void OnMouseDown(MouseEventArgs e)
		{
			base.OnMouseDown(e);
			if (e.Button != MouseButtons.Left || (ModifierKeys & Keys.Alt) == Keys.Alt)
				return;

			int corner = HitCorner(e.Location);
			if (corner != 0)
			{
				resizingCorner = corner;
				hoverCorner = corner;
				resizeStartBounds = PhysicalBounds();
				resizeStartZoom = zoom;
				Cursor = CursorForCorner(corner);
				Invalidate();
				return;
			}

			dragging = true;
			dragStart = PhysicalCursor();
			formStart = PhysicalBounds().Location;
		}

		protected override void OnMouseMove(MouseEventArgs e)
		{
			base.OnMouseMove(e);
			if (resizingCorner != 0)
			{
				ResizeFromCorner();
				return;
			}
			if (dragging)
			{
				Point p = PhysicalCursor();
				ApplyBounds(new Rectangle(
					formStart.X + p.X - dragStart.X,
					formStart.Y + p.Y - dragStart.Y,
					Width, Height));
				return;
			}

			int corner = HitCorner(e.Location);
			if (corner != hoverCorner)
			{
				hoverCorner = corner;
				Cursor = CursorForCorner(corner);
				Invalidate();
			}
		}

		protected override void OnMouseUp(MouseEventArgs e)
		{
			base.OnMouseUp(e);
			if (e.Button == MouseButtons.Left)
			{
				dragging = false;
				resizingCorner = 0;
				// Refresh the hover state: the pointer may have left the corner, or the pin.
				int corner = ClientRectangle.Contains(e.Location) ? HitCorner(e.Location) : 0;
				hoverCorner = corner;
				Cursor = CursorForCorner(corner);
				Invalidate();
			}
		}

		protected override void OnMouseLeave(EventArgs e)
		{
			base.OnMouseLeave(e);
			if (resizingCorner == 0 && hoverCorner != 0)
			{
				hoverCorner = 0;
				Cursor = Cursors.Default;
				Invalidate();
			}
		}

		// Drag a corner: the opposite corner stays put and the image scales proportionally,
		// so the snapshot is never stretched out of shape.
		void ResizeFromCorner()
		{
			Rectangle b = resizeStartBounds;
			Point anchor;
			switch (resizingCorner)
			{
				case 1: anchor = new Point(b.Right, b.Bottom); break;  // dragging top-left
				case 2: anchor = new Point(b.Left, b.Bottom); break;   // dragging top-right
				case 3: anchor = new Point(b.Right, b.Top); break;     // dragging bottom-left
				default: anchor = new Point(b.Left, b.Top); break;     // dragging bottom-right
			}

			Point p = PhysicalCursor();
			float sx = Math.Abs(p.X - anchor.X) / (float)Math.Max(1, b.Width);
			float sy = Math.Abs(p.Y - anchor.Y) / (float)Math.Max(1, b.Height);
			float scale = Math.Max(sx, sy);   // follow whichever axis the pointer pulled further

			float z = ClampZoom(resizeStartZoom * scale);
			Size size = SizeForZoom(z);

			int x, y;
			switch (resizingCorner)
			{
				case 1: x = anchor.X - size.Width; y = anchor.Y - size.Height; break;
				case 2: x = anchor.X; y = anchor.Y - size.Height; break;
				case 3: x = anchor.X - size.Width; y = anchor.Y; break;
				default: x = anchor.X; y = anchor.Y; break;
			}

			zoom = z;
			ApplyBounds(new Rectangle(x, y, size.Width, size.Height));
		}

		protected override void OnMouseDoubleClick(MouseEventArgs e)
		{
			base.OnMouseDoubleClick(e);
			// A click that happens right after the form appears (the mouse-up ending the snapshot
			// drag) must not be taken as a double-click-to-close; same for a double click on a
			// corner, which is where resizing starts.
			if (e.Button == MouseButtons.Left
				&& (DateTime.Now - showtime).TotalMilliseconds > 400
				&& HitCorner(e.Location) == 0)
				ClosePin();
		}

		protected override void OnMouseWheel(MouseEventArgs e)
		{
			base.OnMouseWheel(e);
			if ((ModifierKeys & Keys.Alt) == Keys.Alt)
				SetOpacity((float)(Opacity + (e.Delta > 0 ? 0.05 : -0.05)));
			else
				SetZoom(zoom * (e.Delta > 0 ? 1.1f : 1f / 1.1f));
		}

		protected override bool ProcessCmdKey(ref Message msg, Keys keyData)
		{
			if (keyData == Keys.Escape)
			{
				ClosePin();
				return true;
			}
			if (keyData == (Keys.Control | Keys.C))
			{
				CopyToClipboard();
				return true;
			}
			return base.ProcessCmdKey(ref msg, keyData);
		}

		// Alt held: clicks go through to the window underneath, so the pin can cover a form
		// you still need to interact with
		protected override void WndProc(ref Message m)
		{
			if (m.Msg == WM_NCHITTEST && (ModifierKeys & Keys.Alt) == Keys.Alt)
			{
				m.Result = (IntPtr)HTTRANSPARENT;
				return;
			}
			base.WndProc(ref m);
		}

		protected override void Dispose(bool disposing)
		{
			if (disposing && image != null)
			{
				image.Dispose();
				image = null;
			}
			base.Dispose(disposing);
		}
	}
}
