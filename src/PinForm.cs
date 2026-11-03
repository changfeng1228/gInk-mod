using System;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Drawing.Imaging;
using System.Windows.Forms;
using System.Runtime.InteropServices;

namespace gInk
{
	/// <summary>
	/// A snapshot pinned on top of the desktop, for comparing against whatever is underneath.
	/// Left drag moves it, wheel zooms, Alt+wheel changes opacity, right click for the menu,
	/// double click or Esc closes it. Holding Alt lets clicks fall through to the window below.
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

			// One image pixel per physical screen pixel. The window is created while the thread is
			// DPI aware (see Root.AddPin), so Windows does not stretch it on 125%/150% displays.
			this.ClientSize = new Size(Math.Max(1, image.Width), Math.Max(1, image.Height));
			this.Location = new Point(physicalRect.X, physicalRect.Y);

			BuildMenu();
			showtime = DateTime.Now;
		}

		public void SetZoom(float z)
		{
			zoom = Math.Max(0.1f, Math.Min(8f, z));
			this.ClientSize = new Size(
				Math.Max(1, (int)Math.Round(image.Width * zoom)),
				Math.Max(1, (int)Math.Round(image.Height * zoom)));
			Invalidate();
		}

		ContextMenu menu;
		void BuildMenu()
		{
			menu = new ContextMenu();
			menu.MenuItems.Add(Root.Local.PinMenuCopy, delegate { CopyToClipboard(); });
			menu.MenuItems.Add(Root.Local.PinMenuSave, delegate { SaveAs(); });
			menu.MenuItems.Add("-");
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
		}

		protected override void OnMouseDown(MouseEventArgs e)
		{
			base.OnMouseDown(e);
			if (e.Button == MouseButtons.Left && !(ModifierKeys == Keys.Alt))
			{
				dragging = true;
				dragStart = Cursor.Position;
				formStart = Location;
			}
		}

		protected override void OnMouseMove(MouseEventArgs e)
		{
			base.OnMouseMove(e);
			if (dragging)
			{
				Point p = Cursor.Position;
				Location = new Point(formStart.X + p.X - dragStart.X, formStart.Y + p.Y - dragStart.Y);
			}
		}

		protected override void OnMouseUp(MouseEventArgs e)
		{
			base.OnMouseUp(e);
			if (e.Button == MouseButtons.Left)
				dragging = false;
		}

		protected override void OnMouseDoubleClick(MouseEventArgs e)
		{
			base.OnMouseDoubleClick(e);
			// a click that happens right after the form appears (the mouse-up ending the snapshot
			// drag) must not be taken as a double-click-to-close
			if (e.Button == MouseButtons.Left && (DateTime.Now - showtime).TotalMilliseconds > 400)
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
