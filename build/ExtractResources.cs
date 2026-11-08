
using System;
using System.IO;
using System.Reflection;
using System.Resources;

class ExtractResources
{
    static void Main(string[] args)
    {
        string src = args[0];
        string outdir = args[1];
        Assembly asm = Assembly.LoadFrom(src);
        foreach (string name in asm.GetManifestResourceNames())
        {
            using (Stream s = asm.GetManifestResourceStream(name))
            using (ResourceWriter w = new ResourceWriter(Path.Combine(outdir, name)))
            {
                IResourceReader rr = new ResourceReader(s);
                foreach (System.Collections.DictionaryEntry e in rr)
                    w.AddResource((string)e.Key, e.Value);
                rr.Close();
            }
            Console.WriteLine(name);
        }
    }
}
