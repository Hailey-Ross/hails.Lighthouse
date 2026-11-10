namespace Lighthouse
{
    /// <summary>
    /// Entry point. 7 Days to Die finds this through the IModApi interface when the mod loads.
    /// Status collection and the heartbeat sender are not written yet.
    /// </summary>
    public class LighthouseMod : IModApi
    {
        public void InitMod(Mod modInstance)
        {
            Log.Out("[Lighthouse] Loaded " + modInstance.Name);
        }
    }
}
